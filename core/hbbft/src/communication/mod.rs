use std::{marker::PhantomData, sync::Arc};

use futures::prelude::*;
use futures::sync::{mpsc, oneshot};
use gossip::{GossipMessage, GossipValidator};
use hbbft::{
	crypto::{PublicKey, SecretKey},
	sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, SyncKeyGen},
};
use hbbft_primitives::{AuthorityId, Keypair};
use network::{consensus_gossip as network_gossip, NetworkService};
use network_gossip::ConsensusMessage;
use parity_codec::{Decode, Encode};
use runtime_primitives::traits::{
	Block as BlockT, DigestFor, Hash as HashT, Header as HeaderT, NumberFor, ProvideRuntimeApi,
};

pub use hbbft_primitives::HBBFT_ENGINE_ID;

pub mod gossip;
mod peer;

pub struct NetworkStream {
	inner: Option<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>,
	outer: oneshot::Receiver<mpsc::UnboundedReceiver<network_gossip::TopicNotification>>,
}

impl Stream for NetworkStream {
	type Item = network_gossip::TopicNotification;
	type Error = ();

	fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
		println!("polling in network stream");
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

/// Create a unique topic for a round and set-id combo.
pub(crate) fn round_topic<B: BlockT>(round: u64, set_id: u64) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(format!("{}-{}", set_id, round).as_bytes())
}

pub(crate) fn global_topic<B: BlockT>(epoch: u64) -> B::Hash {
	<<B::Header as HeaderT>::Hashing as HashT>::hash(format!("{}-GLOBAL", epoch).as_bytes())
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
		Self::In {
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

#[derive(Debug, Encode, Decode)]
pub struct SignedMessage<B: BlockT> {
	data: u64,
	sig: u64,
	_phantom: PhantomData<B>,
}

#[derive(Debug, Encode, Decode)]
pub struct Message<B: BlockT> {
	data: u64,
	_phantom: PhantomData<B>,
}

#[derive(Debug)]
pub enum Error {
	Network(String),
}

struct MessageSender<Block: BlockT, N: Network<Block>> {
	locals: Option<(Arc<Keypair>, AuthorityId)>,
	sender: mpsc::UnboundedSender<SignedMessage<Block>>,
	network: N,
}

impl<Block: BlockT, N: Network<Block>> Sink for MessageSender<Block, N> {
	type SinkItem = Message<Block>;
	type SinkError = Error;

	fn start_send(&mut self, mut msg: Message<Block>) -> StartSend<Message<Block>, Error> {
		let signed = SignedMessage {
			data: msg.data,
			sig: 0,
			_phantom: PhantomData,
		};

		let topic = global_topic::<Block>(2);
		self.network.gossip_message(topic, msg.encode(), false);

		// forward the message to the inner sender.
		let _ = self.sender.unbounded_send(signed);

		Ok(AsyncSink::Ready)
	}

	fn poll_complete(&mut self) -> Poll<(), Error> {
		Ok(Async::Ready(()))
	}

	fn close(&mut self) -> Poll<(), Error> {
		// ignore errors since we allow this inner sender to be closed already.
		self.sender.close().or_else(|_| Ok(Async::Ready(())))
	}
}

pub(crate) struct NetworkBridge<B: BlockT, N: Network<B>> {
	service: N,
	validator: Arc<GossipValidator<B>>,
}

impl<B: BlockT, N: Network<B>> NetworkBridge<B, N> {
	pub fn new(service: N, config: crate::NodeConfig) -> Self {
		let validator = Arc::new(GossipValidator::new());
		service.register_validator(validator.clone());

		let topic = global_topic::<B>(1);
		service.register_gossip_message(topic, vec![0u8, 1u8]);

		Self { service, validator }
	}

	pub fn global(
		&self,
	) -> (
		impl Stream<Item = SignedMessage<B>, Error = Error>,
		impl Sink<SinkItem = Message<B>, SinkError = Error>,
	) {
		let topic = global_topic::<B>(1);

		let incoming = self
			.service
			.messages_for(topic)
			.filter_map(|notification| {
				let decoded = GossipMessage::<B>::decode(&mut &notification.message[..]);
				decoded
			})
			.and_then(move |msg| match msg {
				GossipMessage::Message(msg) => {
					println!("gossip msg: {:?}", msg);

					Ok(Some(msg))
				}
			})
			.filter_map(|x| x)
			.map_err(|()| Error::Network(format!("Failed to receive message on unbounded stream")));

		let (tx, out_rx) = mpsc::unbounded();
		let sender = MessageSender::<B, N> {
			locals: None,
			sender: tx,
			network: self.service.clone(),
		};

		let out_rx = out_rx.map_err(move |()| Error::Network(format!("Network Error!")));

		let incoming = incoming.select(out_rx);

		(incoming, sender)
	}
}
