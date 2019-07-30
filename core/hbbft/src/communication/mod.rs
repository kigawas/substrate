use std::sync::Arc;

use futures::prelude::*;
use futures::sync::{mpsc, oneshot};
use hbbft::{
	crypto::{PublicKey, SecretKey},
	sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, SyncKeyGen},
};

use network::{consensus_gossip as network_gossip, NetworkService};
use network_gossip::ConsensusMessage;
use runtime_primitives::traits::{Block as BlockT, DigestFor, NumberFor, ProvideRuntimeApi};

pub use hbbft_primitives::HBBFT_ENGINE_ID;

mod gossip;
mod peer;

pub struct NetworkStream {
	inner: Option<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>,
	outer: oneshot::Receiver<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>,
}

impl Stream for NetworkStream {
	type Item = network_gossip::TopicNotification;
	type Error = ();

	fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
		if let Some(ref mut inner) = self.inner {
			return inner.poll();
		}
		match self.outer.poll() {
			Ok(futures::Async::Ready(mut inner)) => {
				let poll_result = inner.poll();
				self.inner = Some(inner);
				poll_result
			}
			Ok(futures::Async::NotReady) => Ok(futures::Async::NotReady),
			Err(_) => Err(()),
		}
	}
}

pub trait Network<Block: BlockT>: Clone + Send + 'static {
	/// A stream of input messages for a topic.
	type In: Stream<Item = network_gossip::TopicNotification, Error = ()>;

	/// Get a stream of messages for a specific gossip topic.
	fn messages_for(&self, topic: Block::Hash) -> Self::In;

	/// Register a gossip validator.
	fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<Block>>);

	/// Gossip a message out to all connected peers.
	///
	/// Force causes it to be sent to all peers, even if they've seen it already.
	/// Only should be used in case of consensus stall.
	fn gossip_message(&self, topic: Block::Hash, data: Vec<u8>, force: bool);

	/// Register a message with the gossip service, it isn't broadcast right
	/// away to any peers, but may be sent to new peers joining or when asked to
	/// broadcast the topic. Useful to register previous messages on node
	/// startup.
	fn register_gossip_message(&self, topic: Block::Hash, data: Vec<u8>);

	/// Send a message to a bunch of specific peers, even if they've seen it already.
	fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>);

	/// Report a peer's cost or benefit after some action.
	fn report(&self, who: network::PeerId, cost_benefit: i32);

	/// Inform peers that a block with given hash should be downloaded.
	fn announce(&self, block: Block::Hash);
}

impl<B, S, H> Network<B> for Arc<NetworkService<B, S, H>>
where
	B: BlockT,
	S: network::specialization::NetworkSpecialization<B>,
	H: network::ExHashT,
{
	type In = NetworkStream;

	fn messages_for(&self, topic: B::Hash) -> Self::In {
		let (tx, rx) = oneshot::channel();
		self.with_gossip(move |gossip, _| {
			let inner_rx = gossip.messages_for(HBBFT_ENGINE_ID, topic);
			let _ = tx.send(inner_rx);
		});
		NetworkStream {
			outer: rx,
			inner: None,
		}
	}

	fn register_validator(&self, validator: Arc<dyn network_gossip::Validator<B>>) {
		self.with_gossip(move |gossip, context| {
			gossip.register_validator(context, HBBFT_ENGINE_ID, validator)
		})
	}

	fn gossip_message(&self, topic: B::Hash, data: Vec<u8>, force: bool) {
		let msg = ConsensusMessage {
			engine_id: HBBFT_ENGINE_ID,
			data,
		};

		self.with_gossip(move |gossip, ctx| gossip.multicast(ctx, topic, msg, force))
	}

	fn register_gossip_message(&self, topic: B::Hash, data: Vec<u8>) {
		let msg = ConsensusMessage {
			engine_id: HBBFT_ENGINE_ID,
			data,
		};

		self.with_gossip(move |gossip, _| gossip.register_message(topic, msg))
	}

	fn send_message(&self, who: Vec<network::PeerId>, data: Vec<u8>) {
		let msg = ConsensusMessage {
			engine_id: HBBFT_ENGINE_ID,
			data,
		};

		self.with_gossip(move |gossip, ctx| {
			for who in &who {
				gossip.send_message(ctx, who, msg.clone())
			}
		})
	}

	fn report(&self, who: network::PeerId, cost_benefit: i32) {
		self.report_peer(who, cost_benefit)
	}

	fn announce(&self, block: B::Hash) {
		self.announce_block(block)
	}
}
