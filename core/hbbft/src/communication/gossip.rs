use std::{
	collections::{HashMap, VecDeque},
	marker::PhantomData,
	time::{Duration, Instant},
};

use hbbft_primitives::AuthorityId;
use network::consensus_gossip::{self as network_gossip, MessageIntent, ValidatorContext};
use network::{config::Roles, PeerId as NPeerId};
use parity_codec::{Decode, Encode};
use runtime_primitives::traits::{Block as BlockT, NumberFor, Zero};

use futures::prelude::*;
use futures::sync::mpsc;
use log::{debug, error, trace, warn};
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};

use hbbft::{
	crypto::{PublicKeySet, SecretKey, Signature},
	sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, SyncKeyGen},
	{Contribution, NodeIdT},
};
use serde::{Deserialize, Serialize};

use super::peer::PeerId;
use hbbft_primitives::PublicKey;

#[derive(Debug)]
pub(super) enum KeyGenState {
	AwaitingPeers {
		required_peers: Vec<PeerId>,
		available_peers: Vec<PeerId>,
	},
	Generating {
		sync_key_gen: Option<SyncKeyGen<PeerId>>,
		public_key: Option<PublicKey>,
		public_keys: HashMap<PeerId, PublicKey>,

		part_count: u64,
		ack_count: u64,
	},
	Complete {
		sync_key_gen: Option<SyncKeyGen<PeerId>>,
		public_key: Option<PublicKey>,
	},
}

#[cfg_attr(feature = "derive-codec", derive(Encode, Decode))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyGenMessage {
	Part(Part),
	Ack(Ack),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkNodeInfo {
	pub(crate) nid: PeerId,
	pub(crate) pk: PublicKey,
}

type ActiveNetworkInfo = (
	Vec<NetworkNodeInfo>,
	PublicKeySet,
	HashMap<PeerId, PublicKey>,
);

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum NetworkState {
	Unknown(Vec<NetworkNodeInfo>),
	AwaitingPeers(Vec<NetworkNodeInfo>),
	GeneratingKeys(Vec<NetworkNodeInfo>, HashMap<PeerId, PublicKey>),
	Active(ActiveNetworkInfo),
}

#[derive(Debug, Encode, Decode)]
pub enum GossipMessage<Block: BlockT> {
	Message(super::SignedMessage<Block>),
}

pub struct GossipValidator<Block: BlockT> {
	inner: parking_lot::RwLock<u64>,
	_phantom: PhantomData<Block>,
}

impl<Block: BlockT> GossipValidator<Block> {
	pub fn new() -> Self {
		Self {
			inner: parking_lot::RwLock::new(1),
			_phantom: PhantomData,
		}
	}
}

impl<Block: BlockT> network_gossip::Validator<Block> for GossipValidator<Block> {
	fn new_peer(&self, context: &mut dyn ValidatorContext<Block>, who: &NPeerId, _roles: Roles) {
		println!("in new peer");
	}

	fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<Block>, who: &NPeerId) {
		println!("in peer disconnected");
	}

	fn validate(
		&self,
		context: &mut dyn ValidatorContext<Block>,
		who: &NPeerId,
		data: &[u8],
	) -> network_gossip::ValidationResult<Block::Hash> {
		println!("in validate {:?}", data);
		let topic = super::global_topic::<Block>(1);
		network_gossip::ValidationResult::ProcessAndKeep(topic)
	}

	fn message_allowed<'a>(
		&'a self,
	) -> Box<dyn FnMut(&NPeerId, MessageIntent, &Block::Hash, &[u8]) -> bool + 'a> {
		let (inner, do_rebroadcast) = {
			use parking_lot::RwLockWriteGuard;

			let mut inner = self.inner.write();
			*inner = 2;

			let now = Instant::now();
			let do_rebroadcast = false;
			// downgrade to read-lock.
			(RwLockWriteGuard::downgrade(inner), do_rebroadcast)
		};

		Box::new(move |who, intent, topic, mut data| {
			println!("inner value {:?}", inner);
			println!("in message allowed {:?}", data);
			true
		})
	}

	fn message_expired<'a>(&'a self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'a> {
		let inner = self.inner.read();
		Box::new(move |topic, mut data| {
			println!("in message expired {:?}", data);
			match *inner {
				1 => false,
				_ => true,
			}
		})
	}
}
