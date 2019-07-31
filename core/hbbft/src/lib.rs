use std::sync::Arc;

use client::blockchain::HeaderBackend;
use client::{
	backend::Backend, error::Error as ClientError, error::Result, BlockchainEvents, CallExecutor,
	Client,
};
use consensus_common::SelectChain;
use futures::future::Loop as FutureLoop;
use futures::prelude::*;
use futures::sync::mpsc;
use hbbft_primitives::HbbftApi;
use inherents::InherentDataProviders;
use log::{debug, info, warn};
use parity_codec::Encode;
use runtime_primitives::generic::BlockId;
use runtime_primitives::traits::{Block as BlockT, DigestFor, NumberFor, ProvideRuntimeApi};
use serde_json;
use substrate_primitives::{ed25519, Blake2Hasher, Pair, H256};
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG, CONSENSUS_INFO, CONSENSUS_WARN};
use tokio_executor::DefaultExecutor;

pub use communication::Network;

#[cfg(test)]
mod tests;

mod communication;

pub fn run_key_gen<Block: BlockT<Hash = H256>, N>(
	network: N,
) -> Result<impl Future<Item = (), Error = ()> + Send + 'static>
where
	N: Network<Block> + Send + Sync + 'static,
{
	let initial_state = 1;
	let key_gen_work = futures::future::loop_fn(initial_state, move |params| {
		let validator = communication::GossipValidator::new();
		network.register_validator(Arc::new(validator));

		let topic = communication::global_topic::<Block>(0);
		network.gossip_message(topic, vec![0u8, 1u8], true);
		println!("broadcast msg ok");

		Ok(FutureLoop::Break(()))
	});

	Ok(key_gen_work)
}
