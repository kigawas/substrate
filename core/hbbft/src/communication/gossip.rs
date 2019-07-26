
use runtime_primitives::traits::{NumberFor, Block as BlockT, Zero};
use network::consensus_gossip::{self as network_gossip, MessageIntent, ValidatorContext};
use network::{config::Roles, PeerId};
use parity_codec::{Encode, Decode};
use hbbft_primitives::AuthorityId;

use substrate_telemetry::{telemetry, CONSENSUS_DEBUG};
use log::{trace, debug, warn, error};
use futures::prelude::*;
use futures::sync::mpsc;


use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use hbbft::{
    {NodeIdT, Contribution},
    sync_key_gen::{Ack, AckOutcome, Part, PartOutcome, SyncKeyGen},
};

use hbbft_primitives::{PublicKey};


#[derive(Debug)]
pub(super) enum State {
    AwaitingPeers {
        required_peers: Vec<PeerId>,
        available_peers: Vec<PeerId>,
    },
    Generating {
        sync_key_gen: Option<SyncKeyGen<PeerId>>,
        public_key: Option<PublicKey>,
        public_keys: HashMap<PeerId, PublicKey>,

        part_count: usize,
        ack_count: usize,
    },
    Complete {
        sync_key_gen: Option<SyncKeyGen<PeerId>>,
        public_key: Option<PublicKey>,
    },
}


// fn handle_ack<N: NodeId>(
//     nid: &N,
//     ack: Ack,
//     ack_count: &mut usize,
//     sync_key_gen: &mut SyncKeyGen<N>,
// ) {
//     trace!("KEY GENERATION: Handling ack from '{:?}'...", nid);
//     let ack_outcome = sync_key_gen
//         .handle_ack(nid, ack.clone())
//         .expect("Failed to handle Ack.");
//     match ack_outcome {
//         AckOutcome::Invalid(fault) => error!("Error handling ack: '{:?}':\n{:?}", ack, fault),
//         AckOutcome::Valid => *ack_count += 1,
//     }
// }

// fn handle_queued_acks<C: Contribution, N: NodeId>(
//     ack_queue: &SegQueue<(N, Ack)>,
//     part_count: usize,
//     ack_count: &mut usize,
//     sync_key_gen: &mut SyncKeyGen<N>,
//     peers: &Peers<C, N>,
// ) {
//     if part_count == peers.count_validators() + 1 {
//         trace!("KEY GENERATION: Handling queued acks...");

//         debug!("   Peers complete: {}", sync_key_gen.count_complete());
//         debug!("   Part count: {}", part_count);
//         debug!("   Ack count: {}", ack_count);

//         while let Some((nid, ack)) = ack_queue.try_pop() {
//             handle_ack(&nid, ack, ack_count, sync_key_gen);
//         }
//     }
// }



#[derive(Debug, Encode, Decode)]
pub(super) enum GossipMessage {

}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "derive-codec", derive(Encode, Decode))]
pub enum KeyGenMessage {
    Part(Part),
    Ack(Ack),
}
