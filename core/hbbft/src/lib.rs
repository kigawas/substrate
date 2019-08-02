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
use hbbft::crypto::{PublicKey, SecretKey, SignatureShare};
use hbbft_primitives::HbbftApi;
use inherents::InherentDataProviders;
use log::{debug, info, warn};
use network;
use parity_codec::Encode;
use runtime_primitives::generic::BlockId;
use runtime_primitives::traits::{Block as BlockT, DigestFor, NumberFor, ProvideRuntimeApi};
use serde_json;
use substrate_primitives::H256;
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG, CONSENSUS_INFO, CONSENSUS_WARN};
use tokio_executor::DefaultExecutor;

pub use communication::Network;

#[cfg(test)]
mod tests;

mod communication;

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

pub fn run_key_gen<Block: BlockT<Hash = H256>, N>(
	network: N,
) -> Result<impl Future<Item = (), Error = ()> + Send + 'static>
where
	N: Network<Block> + Send + Sync + 'static,
{
	let initial_state = 1;
	let config = NodeConfig {
		local_key: None,
		name: None,
	};
	let bridge = communication::NetworkBridge::new(network.clone(), config);
	println!("setup network bridge OK");

	let key_gen_work = futures::future::loop_fn(initial_state, move |params| {
		println!("aaa");

		Ok(FutureLoop::Break(()))
	});

	let topic = communication::global_topic::<Block>(1);
	let on_message = network.messages_for(topic).for_each(|notification| {
		println!("noti {:?}", notification);

		Ok(())
	});

	// let (tx, out_rx) = mpsc::unbounded();
	// let on_message = on_message.select(out_rx);

	// Ok(key_gen_work.select2(on_message).then(|_| Ok(())))
	// Ok(key_gen_work)
	Ok(on_message)
}
