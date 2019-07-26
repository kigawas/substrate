use std::collections::{HashMap, VecDeque};

use hbbft_primitives::{PublicKey};
use network::{config::Roles, PeerId};


enum PeerPhase {
	Handshaking,
	PendingJoin {
		pk: PublicKey
	},
	EstablishedObserver{
		pk: PublicKey
	},
	EstablishedValidator{
		pk: PublicKey
	},
}


struct PeerInfo{
	phase: PeerPhase,
}

impl PeerInfo {
	fn new(public_key: Option<PublicKey>) -> Self {
		let phase = match public_key {
			Some(pk) => PeerPhase::EstablishedValidator {pk},
			None => PeerPhase::Handshaking
		};

		PeerInfo {
			phase
		}
	}
}

struct Peers{
	map: HashMap<PeerId, PeerInfo>
}

impl Default for Peers {
	fn default() -> Self{
		Peers { map: HashMap::new() }
	}
}

impl Peers {
	fn add(&mut self, who: PeerId, public_key: Option<PublicKey>) {
		self.map.insert(who, PeerInfo::new(public_key));
	}
	fn del(&mut self, who: &PeerId){
		self.map.remove(who);
	}
}
