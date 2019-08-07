use std::{
	collections::VecDeque,
	fmt::Debug,
	marker::PhantomData,
	sync::Arc,
	time::{Duration, Instant},
};

use client::blockchain::HeaderBackend;
use client::{
	backend::Backend, error::Error as ClientError, error::Result, BlockchainEvents, CallExecutor,
	Client,
};
use codec::{Decode, Encode};
use consensus_common::SelectChain;
use futures::{future::Loop as FutureLoop, prelude::*, stream::Fuse, sync::mpsc};
use hbbft::crypto::{PublicKey, SecretKey, SignatureShare};
use hbbft_primitives::HbbftApi;
use inherents::InherentDataProviders;
use log::{debug, info, warn};
use network;
use runtime_primitives::generic::BlockId;
use runtime_primitives::traits::{Block as BlockT, DigestFor, NumberFor, ProvideRuntimeApi};
use serde_json;
use substrate_primitives::H256;
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG, CONSENSUS_INFO, CONSENSUS_WARN};
use tokio_executor::DefaultExecutor;
use tokio_timer::Interval;

pub use communication::Network;

#[cfg(test)]
mod tests;

mod communication;
use communication::{gossip::GossipMessage, SignedMessage};

#[derive(Clone)]
pub struct NodeConfig {
	pub local_key: Option<Arc<SecretKey>>,
	name: Option<String>,
}

impl NodeConfig {
	pub fn name(&self) -> &str {
		self.name
			.as_ref()
			.map(|s| s.as_str())
			.unwrap_or("<unknown>")
	}
}

struct Worker<Block: BlockT, S, M> {
	incoming: Fuse<S>,
	check_pending: Interval,
	ready: VecDeque<M>,
	_phantom: PhantomData<Block>,
}

impl<Block, S, M> Worker<Block, S, M>
where
	Block: BlockT,
	S: Stream<Item = M, Error = communication::Error>,
	M: Debug,
{
	pub fn new(stream: S) -> Self {
		let now = Instant::now();
		let dur = Duration::from_secs(5);
		let check_pending = Interval::new(now + dur, dur);

		Self {
			incoming: stream.fuse(),
			check_pending,
			ready: VecDeque::new(),
			_phantom: PhantomData,
		}
	}
}

impl<Block, S, M> Stream for Worker<Block, S, M>
where
	Block: BlockT,
	S: Stream<Item = M, Error = communication::Error>,
	M: Debug,
{
	type Item = M;
	type Error = communication::Error;

	fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
		loop {
			match self.incoming.poll()? {
				Async::Ready(None) => return Ok(Async::Ready(None)),
				Async::Ready(Some(input)) => {
					println!("worker poll {:?}", input);
					let ready = &mut self.ready;
					ready.push_back(input);
				}
				Async::NotReady => break,
			}
		}

		while let Async::Ready(Some(p)) = self
			.check_pending
			.poll()
			.map_err(|e| communication::Error::Network("pending err".to_string()))?
		{
			println!("Check pending {:?}", p);
		}

		if let Some(ready) = self.ready.pop_front() {
			return Ok(Async::Ready(Some(ready)));
		}

		if self.incoming.is_done() {
			println!("worker incoming done");
			Ok(Async::Ready(None))
		} else {
			println!("worker incoming not ready");
			Ok(Async::NotReady)
		}
	}
}

pub fn run_key_gen<Block: BlockT<Hash = H256>, N>(
	network: N,
) -> Result<impl Future<Item = (), Error = ()> + Send + 'static>
where
	Block::Hash: Ord,
	N: Network<Block> + Send + Sync + 'static,
	N::In: Send + 'static,
{
	let config = NodeConfig {
		local_key: None,
		name: None,
	};
	let bridge = communication::NetworkBridge::new(network, config.clone());
	let initial_state = 1;

	let key_gen_work = futures::future::loop_fn(initial_state, move |params| {
		let (mut incoming, outgoing) = bridge.global();

		let mut incoming = Worker::<Block, _, SignedMessage<Block>>::new(incoming);
		let mut incoming = incoming.map_err(|_| ClientError::Msg("error when polling".to_string()));

		let poll_voter = futures::future::poll_fn(move || -> Result<_> {
			println!("in poll_fn");

			match incoming.poll() {
				Ok(r) => match r {
					Async::Ready(Some(item)) => {
						println!("incoming polling ready {:?}", item);
						return Ok(Async::Ready(Some(item)));
					}
					Async::Ready(None) => {
						println!("incoming polling None");
						return Ok(Async::Ready(None));
					}
					Async::NotReady => {
						println!("incoming polling Not ready");
						return Ok(Async::NotReady);
					}
				},
				Err(e) => return Err(e),
			}
		});

		poll_voter.then(move |res| {
			println!("poll voter res {:?}", res);
			match res {
				Ok(_) => Ok(FutureLoop::Continue((1))), // Ready(None)
				Err(_) => Ok(FutureLoop::Break(())),
			}
		})
	});
	Ok(key_gen_work)
}
