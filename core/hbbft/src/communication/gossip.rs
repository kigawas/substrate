use codec::{Decode, Encode};
use futures::prelude::*;
use futures::sync::mpsc;
use hbbft::{
	crypto::{PublicKeySet, SecretKey, Signature},
	dynamic_honey_badger::{DynamicHoneyBadger, JoinPlan},
	sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, SyncKeyGen},
	{Contribution, NodeIdT},
};
use hbbft_primitives::AuthorityId;
use log::{debug, error, trace, warn};
use network::consensus_gossip::{self as network_gossip, MessageIntent, ValidatorContext};
use network::{config::Roles, PeerId};
use runtime_primitives::traits::{Block as BlockT, NumberFor, Zero};
use serde::{Deserialize, Serialize};
use std::{
	collections::{HashMap, VecDeque},
	marker::PhantomData,
	time::{Duration, Instant},
};
use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};

use super::{
	message::{InstanceId, KeyGenMessage},
	peer::{NodeId, Peers},
};
use hbbft_primitives::PublicKey;

#[derive(Debug)]
pub(super) enum KeyGenState {
	AwaitingPeers,
	Generating,
	Complete,
}

// #[cfg_attr(feature = "derive-codec", derive(Encode, Decode))]
// #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// pub enum KeyGenMessage {
// 	Part(Part),
// 	Ack(Ack),
// }

// #[derive(Clone, Debug, Serialize, Deserialize)]
// pub struct NetworkNodeInfo {
// 	pub(crate) nid: PeerId,
// 	pub(crate) pk: PublicKey,
// }

// type ActiveNetworkInfo = (
// 	Vec<NetworkNodeInfo>,
// 	PublicKeySet,
// 	HashMap<PeerId, PublicKey>,
// );

// #[derive(Debug, Serialize, Deserialize)]
// pub(super) enum NetworkState {
// 	Unknown(Vec<NetworkNodeInfo>),
// 	AwaitingPeers(Vec<NetworkNodeInfo>),
// 	GeneratingKeys(Vec<NetworkNodeInfo>, HashMap<PeerId, PublicKey>),
// 	Active(ActiveNetworkInfo),
// }

#[derive(Debug, Encode, Decode)]
pub enum GossipMessage<Block: BlockT> {
	KeyGen(InstanceId, KeyGenMessage),
	// JoinPlan(JoinPlan<NodeId>),
	Message(super::SignedMessage<Block>),
}

#[derive(Debug)]
struct Inner {
	dhb: u64,
	peers: Peers,
}

impl Default for Inner {
	fn default() -> Self {
		Self {
			dhb: 0,
			peers: Peers::default(),
		}
	}
}

pub struct GossipValidator<Block: BlockT> {
	inner: parking_lot::RwLock<Inner>,
	// dhb:
	_phantom: PhantomData<Block>,
}

impl<Block: BlockT> GossipValidator<Block> {
	pub fn new() -> Self {
		Self {
			inner: parking_lot::RwLock::new(Inner::default()),
			_phantom: PhantomData,
		}
	}
}

impl<Block: BlockT> network_gossip::Validator<Block> for GossipValidator<Block> {
	fn new_peer(&self, context: &mut dyn ValidatorContext<Block>, who: &PeerId, _roles: Roles) {
		// 1. add peer info
		{
			let mut inner = self.inner.write();
			inner.peers.add(NodeId::from(who.clone()));
			println!("{:?}", inner.peers);
		}
		// 2. key gen?
		// awaiting peers -> send all my peer public keys to peer
		// generating ->
		// finished ->
		println!("in new peer");
	}

	fn peer_disconnected(&self, _context: &mut dyn ValidatorContext<Block>, who: &PeerId) {
		println!("in peer disconnected");
	}

	fn validate(
		&self,
		context: &mut dyn ValidatorContext<Block>,
		who: &PeerId,
		data: &[u8],
	) -> network_gossip::ValidationResult<Block::Hash> {
		println!("in validate {:?}", data);
		let topic = super::global_topic::<Block>(1);
		network_gossip::ValidationResult::ProcessAndKeep(topic)
	}

	fn message_allowed<'a>(
		&'a self,
	) -> Box<dyn FnMut(&PeerId, MessageIntent, &Block::Hash, &[u8]) -> bool + 'a> {
		let (inner, do_rebroadcast) = {
			use parking_lot::RwLockWriteGuard;

			let mut inner = self.inner.write();
			inner.dhb = 2;

			let now = Instant::now();
			let do_rebroadcast = false;
			// downgrade to read-lock.
			(RwLockWriteGuard::downgrade(inner), do_rebroadcast)
		};

		Box::new(move |who, intent, topic, mut data| {
			println!("message_allowed  inner: {:?}, data: {:?}", inner, data);
			false
		})
	}

	fn message_expired<'a>(&'a self) -> Box<dyn FnMut(Block::Hash, &[u8]) -> bool + 'a> {
		let inner = self.inner.read();
		Box::new(move |topic, mut data| {
			println!("message_expired {:?}", data);
			// match *inner {
			// 	1 => false,
			// 	_ => true,
			// }
			true
		})
	}
}
