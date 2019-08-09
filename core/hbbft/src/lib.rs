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
mod periodic_stream;

use communication::{gossip::GossipMessage, Message, NetworkBridge, SignedMessage};
use periodic_stream::PeriodicStream;

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

fn global_comm<Block, N>(
	bridge: &NetworkBridge<Block, N>,
) -> (
	impl Stream<Item = SignedMessage<Block>, Error = ClientError>,
	impl Sink<SinkItem = Message<Block>, SinkError = ClientError>,
)
where
	Block: BlockT<Hash = H256>,
	N: Network<Block>,
{
	let (global_in, global_out) = bridge.global();
	let global_in = PeriodicStream::<Block, _, SignedMessage<Block>>::new(global_in);

	let global_in = global_in.map_err(|_| ClientError::Msg("error global in".to_string()));
	let global_out = global_out.sink_map_err(|_| ClientError::Msg("error global out".to_string()));

	(global_in, global_out)
}

pub fn run_key_gen<Block, N>(
	network: N,
) -> Result<impl Future<Item = (), Error = ()> + Send + 'static>
where
	Block: BlockT<Hash = H256>,
	Block::Hash: Ord,
	N: Network<Block> + Send + Sync + 'static,
	N::In: Send + 'static,
{
	let config = NodeConfig {
		local_key: None,
		name: None,
	};
	let bridge = NetworkBridge::new(network, config.clone());
	let initial_state = 1;

	let key_gen_work = futures::future::loop_fn(initial_state, move |params| {
		let (mut incoming, mut outgoing) = global_comm(&bridge);

		let poll_voter = futures::future::poll_fn(move || -> Result<_> {
			println!("in poll_fn");

			match incoming.poll() {
				Ok(r) => match r {
					Async::Ready(Some(item)) => {
						println!("incoming polling ready {:?}", item);
						// let message = Message {
						// 	data: 100,
						// 	_phantom: PhantomData,
						// };
						// outgoing.start_send(message);
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
