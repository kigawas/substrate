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
use substrate_primitives::H256;
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG, CONSENSUS_INFO, CONSENSUS_WARN};
use tokio_timer::Interval;

use crate::communication;

pub struct PeriodicStream<Block: BlockT, S, M> {
	incoming: Fuse<S>,
	check_pending: Interval,
	ready: VecDeque<M>,
	_phantom: PhantomData<Block>,
}

impl<Block, S, M> PeriodicStream<Block, S, M>
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

impl<Block, S, M> Stream for PeriodicStream<Block, S, M>
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
