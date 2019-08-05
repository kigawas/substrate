use std::sync::Arc;
use std::time::{Duration, Instant};

use client::blockchain::HeaderBackend;
use client::{
	backend::Backend, error::Error as ClientError, error::Result, BlockchainEvents, CallExecutor,
	Client,
};
use consensus_common::SelectChain;
use futures::{future::Loop as FutureLoop, prelude::*, sync::mpsc};
use futures_timer::Interval;
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

	let dur = Duration::from_secs(3);
	// let stream = Interval::new(dur).map(|()| println!("prints every four seconds"));

	let key_gen_work = futures::future::loop_fn(initial_state, move |params| {
		println!("key gen work");
		let bridge = bridge.clone();
		// let p = incoming.poll();

		// println!("{:?}", p);

		let poll_voter = futures::future::poll_fn(move || -> Result<_> {
			// let poll_voter = Interval::new(dur).map(|_| {
			println!("in poll loop");
			// let bridge = bridge.clone();
			let (mut incoming, outgoing) = bridge.global();

			match incoming.poll() {
				Ok(r) => match r {
					Async::Ready(Some(item)) => {
						println!("{:?}", item);
						return Ok(Async::Ready(Some(item)));
					}
					Async::Ready(None) => {
						println!("None");
						return Ok(Async::Ready(None));
					}
					Async::NotReady => {
						println!("Not ready");
						return Ok(Async::NotReady);
					}
				},
				Err(_) => return Err(ClientError::Msg("error when polling".to_string())),
			}
		});

		poll_voter.then(move |res| {
			println!("poll voter res {:?}", res);
			match res {
				Ok(_) => Ok(FutureLoop::Continue((1))),
				Err(_) => Ok(FutureLoop::Break(())),
			}
		})
	});

	// let dummy = futures::future::loop_fn(initial_state, move |params| {
	// 	println!("dummy work");
	// 	Ok(FutureLoop::Break(()))
	// });

	// let poll_voter = poll_voter.then(|_| Ok(FutureLoop::Break(())));
	// let key_gen_work = key_gen_work
	// 	.map(|_| ())
	// 	.map_err(|e| println!("key gen error"));
	// .then(move |res| Ok(FutureLoop::Continue((1))))
	// .map(|_| ());
	// Ok(dummy)
	Ok(key_gen_work)
}
