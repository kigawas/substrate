use codec::{Decode, Encode, Error as CodecError, Input};
use hbbft_primitives::PublicKey;
use log::{debug, error, trace, warn};
use multihash::Multihash as PkHash;
use network::{config::Roles, PeerId};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::{min, Ordering};
use std::collections::{HashMap, VecDeque};
use std::convert::From;

pub(crate) type Index = u32;

#[derive(Debug)]
pub(crate) struct PeerInfo {
	pub index: Index,
}

impl Default for PeerInfo {
	fn default() -> Self {
		Self { index: 0 }
	}
}

impl PeerInfo {
	fn new(index: Index) -> Self {
		Self { index }
	}
}

#[derive(Debug)]
pub(crate) struct Peers {
	map: HashMap<PeerId, PeerInfo>,
}

impl Default for Peers {
	fn default() -> Self {
		Self::default()
	}
}

impl Peers {
	pub fn add(&mut self, who: PeerId) {
		self.map.insert(who, PeerInfo::default());
	}

	pub fn del(&mut self, who: &PeerId) {
		self.map.remove(who);
	}

	pub fn set_index(&mut self, who: &PeerId, index: Index) {
		let peer = self.map.get_mut(who).expect("Peer not found!");
		peer.index = index;
	}
}
