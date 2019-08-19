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
use log::{debug, error, info, warn};
use network;
use runtime_primitives::generic::BlockId;
use runtime_primitives::traits::{Block as BlockT, DigestFor, NumberFor, ProvideRuntimeApi};
use substrate_primitives::{Blake2Hasher, H256};
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG, CONSENSUS_INFO, CONSENSUS_WARN};
use tokio_executor::DefaultExecutor;
use tokio_timer::Interval;

pub use communication::Network;

#[cfg(test)]
mod tests;

mod communication;
mod periodic_stream;
mod shared_state;

use communication::{gossip::GossipMessage, Message, NetworkBridge, SignedMessage};
use periodic_stream::PeriodicStream;
use shared_state::{load_persistent, set_index, SharedState};

#[derive(Clone)]
pub struct NodeConfig {
	pub threshold: u32,
	pub parties: u32,
	pub name: Option<String>,
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

struct Environment<Block: BlockT, N: Network<Block>> {
	pub config: NodeConfig,
	pub bridge: NetworkBridge<Block, N>,
}

#[must_use]
struct KeyGenWork<B, E, Block: BlockT, N: Network<Block>, RA> {
	client: Arc<Client<B, E, Block, RA>>,
	key_gen: Box<dyn Future<Item = (), Error = ClientError> + Send>,
	env: Arc<Environment<Block, N>>,
}

impl<B, E, Block, N, RA> KeyGenWork<B, E, Block, N, RA>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
	Block: BlockT<Hash = H256>,
	Block::Hash: Ord,
	N: Network<Block> + Send + Sync + 'static,
	N::In: Send + 'static,
	RA: Send + Sync + 'static,
{
	fn new(
		client: Arc<Client<B, E, Block, RA>>,
		config: NodeConfig,
		bridge: NetworkBridge<Block, N>,
	) -> Self {
		let env = Arc::new(Environment { config, bridge });
		let mut work = Self {
			// `voter` is set to a temporary value and replaced below when
			// calling `rebuild_voter`.
			client,
			key_gen: Box::new(futures::empty()) as Box<_>,
			env,
		};
		work.rebuild();
		work
	}

	fn rebuild(&mut self) {
		let should_rebuild = true;
		if should_rebuild {
			let (mut incoming, mut outgoing) = global_comm(&self.env.bridge);
			let poll_key_gen = futures::future::poll_fn(move || -> Result<_> {
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
							return Ok(Async::Ready(()));
						}
						Async::Ready(None) => {
							println!("incoming polling None");
							return Ok(Async::Ready(()));
						}
						Async::NotReady => {
							println!("incoming polling Not ready");
							return Ok(Async::NotReady);
						}
					},
					Err(e) => return Err(e),
				}
			});
			self.key_gen = Box::new(poll_key_gen);
		} else {
			self.key_gen = Box::new(futures::empty());
		}
	}
}

impl<B, E, Block, N, RA> Future for KeyGenWork<B, E, Block, N, RA>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
	Block: BlockT<Hash = H256>,
	Block::Hash: Ord,
	N: Network<Block> + Send + Sync + 'static,
	N::In: Send + 'static,
	RA: Send + Sync + 'static,
{
	type Item = ();
	type Error = ClientError;

	fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
		match self.key_gen.poll() {
			Ok(Async::NotReady) => {
				return Ok(Async::NotReady);
			}
			Ok(Async::Ready(())) => {
				// voters don't conclude naturally
				return Ok(Async::Ready(()));
			}
			Err(e) => {
				// return inner observer error
				println!("error from poll {:?}", e);
				return Err(e);
			}
		}
	}
}

pub fn init_shared_state<B, E, Block, RA>(client: Arc<Client<B, E, Block, RA>>) -> SharedState
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
	Block: BlockT<Hash = H256>,
	Block::Hash: Ord,
	RA: Send + Sync + 'static,
{
	let persistent_data: SharedState = load_persistent(&**client.backend()).unwrap();
	persistent_data
	// set_index(&**client.backend(), persistent_data.current_index + 1);
}

pub fn run_key_gen<B, E, Block, N, RA>(
	client: Arc<Client<B, E, Block, RA>>,
	network: N,
) -> Result<impl Future<Item = (), Error = ()> + Send + 'static>
where
	B: Backend<Block, Blake2Hasher> + 'static,
	E: CallExecutor<Block, Blake2Hasher> + 'static + Clone + Send + Sync,
	Block: BlockT<Hash = H256>,
	Block::Hash: Ord,
	N: Network<Block> + Send + Sync + 'static,
	N::In: Send + 'static,
	RA: Send + Sync + 'static,
{
	let config = NodeConfig {
		name: None,
		threshold: 1,
		parties: 3,
	};

	// let persistent_data: SharedState = load_persistent(&**client.backend()).unwrap();
	// println!("{:?}", persistent_data);
	// set_index(&**client.backend(), persistent_data.current_index + 1);

	let bridge = NetworkBridge::new(network, config.clone());

	let key_gen_work = KeyGenWork::new(client, config, bridge).map_err(|e| error!("Error {:?}", e));
	Ok(key_gen_work)
}
