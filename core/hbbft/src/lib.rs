use std::{
	marker::PhantomData,
	sync::Arc,
	time::{Duration, Instant},
};

use client::blockchain::HeaderBackend;
use client::{
	backend::Backend, error::Error as ClientError, error::Result, BlockchainEvents, CallExecutor,
	Client,
};
use consensus_common::SelectChain;
use futures::{future::Loop as FutureLoop, prelude::*, sync::mpsc};
use hbbft::crypto::{PublicKey, SecretKey, SignatureShare};
use hbbft_primitives::HbbftApi;
use inherents::InherentDataProviders;
use log::{debug, info, warn};
use network;
use parity_codec::{Decode, Encode};
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
use communication::gossip::GossipMessage;

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

// struct Worker<Block: BlockT, S: Stream> {
// 	incoming: S,
// 	check_pending: Interval,
// 	_phantom: PhantomData<Block>,
// }

// impl<Block: BlockT, S: Stream> Worker<Block, S> {
// 	fn new(stream: S) -> Self {
// 		let now = Instant::now();
// 		let dur = Duration::from_secs(5);
// 		let check_pending = Interval::new(now + dur, dur);

// 		Self {
// 			incoming: stream,
// 			check_pending,
// 			_phantom: PhantomData,
// 		}
// 	}
// }

// impl<Block: BlockT, S: Stream> Stream for Worker<Block, S> {
// 	type Item = ();
// 	type Error = ClientError;

// 	fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {}
// }

pub fn run_key_gen<Block: BlockT<Hash = H256>, N>(
	network: N,
) -> Result<impl Future<Item = (), Error = ()> + Send + 'static>
where
	N: Network<Block> + Send + Sync + 'static,
{
	let config = NodeConfig {
		local_key: None,
		name: None,
	};
	let bridge = communication::NetworkBridge::new(network, config.clone());
	let initial_state = 1;
	let now = Instant::now();
	let dur = Duration::from_secs(3);

	// let key_gen_work = futures::future::loop_fn(initial_state, move |params| {
	// 	println!("key gen work");
	// 	let bridge = bridge.clone();
	// 	let mut pending = Interval::new(now + dur, dur);

	// 	let poll_voter = futures::future::poll_fn(move || -> Result<_> {
	// 		while let Async::Ready(Some(p)) = pending
	// 			.poll()
	// 			.map_err(|e| ClientError::Msg("pending err".to_string()))?
	// 		{
	// 			println!("pending interval res {:?}", p);
	// 		}

	// 		// let poll_voter = Interval::new(dur).map(|_| {

	// 		println!("in poll loop");
	// 		// let bridge = bridge.clone();
	// 		let (mut incoming, outgoing) = bridge.global();

	// 		match incoming.poll() {
	// 			Ok(r) => match r {
	// 				Async::Ready(Some(item)) => {
	// 					println!("{:?}", item);
	// 					return Ok(Async::Ready(Some(item)));
	// 				}
	// 				Async::Ready(None) => {
	// 					println!("None");
	// 					return Ok(Async::Ready(None));
	// 				}
	// 				Async::NotReady => {
	// 					println!("Not ready");
	// 					return Ok(Async::NotReady);
	// 				}
	// 			},
	// 			Err(_) => return Err(ClientError::Msg("error when polling".to_string())),
	// 		}
	// 	});

	// 	poll_voter.then(move |res| {
	// 		println!("poll voter res {:?}", res);
	// 		match res {
	// 			Ok(_) => Ok(FutureLoop::Continue((1))),
	// 			Err(_) => Ok(FutureLoop::Break(())),
	// 		}
	// 	})
	// });
	// Ok(key_gen_work)

	let dummy = futures::future::loop_fn(initial_state, move |params| {
		println!("dummy work");
		Ok(FutureLoop::Break(()))
	});

	Ok(dummy)
}
