use codec::{Decode, Encode};
use futures::prelude::*;
use futures::sync::mpsc;
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
	message::{KeyGenMessage, SignMessage},
	peer::{Index as PeerIndex, PeerInfo, Peers},
};
use hbbft_primitives::PublicKey;

#[derive(Debug)]
pub(super) enum KeyGenState {
	AwaitingPeers,
	Generating,
	Complete,
}

#[derive(Debug, Encode, Decode)]
pub enum GossipMessage<Block: BlockT> {
	KeyGen(KeyGenMessage),
	Message(super::SignedMessage<Block>),
}

#[derive(Debug)]
struct Inner {
	local_peer_info: PeerInfo,
	peers: Peers,
}
impl Inner {
	fn set_local_index(&mut self, index: PeerIndex) {
		self.local_peer_info.index = index;
	}

	fn set_peer_index(&mut self, who: &PeerId, index: PeerIndex) {
		self.peers.set_index(who, index);
	}
}

impl Default for Inner {
	fn default() -> Self {
		Self {
			local_peer_info: PeerInfo::default(),
			peers: Peers::default(),
		}
	}
}

pub struct GossipValidator<Block: BlockT> {
	inner: parking_lot::RwLock<Inner>,
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
			inner.peers.add(who.clone());
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
