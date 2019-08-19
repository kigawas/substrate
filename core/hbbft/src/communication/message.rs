use bincode;
use codec::{Decode, Encode, Error as CodecError, Input};
use rand::rngs::{OsRng, StdRng};
use rand::{RngCore, SeedableRng};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::peer::Index as PeerIndex;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct BroadCastMessage {
	index: PeerIndex,
	msg: u32,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct DecommitMessage {
	index: PeerIndex,
	msg: u32,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum KeyGenMessage {
	BroadCastMessage,
	DecommitMessage,
}

pub enum SignMessage {}

impl Encode for KeyGenMessage {
	fn encode(&self) -> Vec<u8> {
		let encoded = bincode::serialize(&self).unwrap();
		let bytes = encoded.as_slice();
		Encode::encode(&bytes)
	}
}

impl Decode for KeyGenMessage {
	fn decode<I: Input>(value: &mut I) -> Result<Self, CodecError> {
		let decoded: Vec<u8> = Decode::decode(value)?;
		let bytes = decoded.as_slice();
		Ok(bincode::deserialize(bytes).unwrap())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use std::collections::BTreeMap;
	#[test]
	fn test_message_encode_decode() {
		// let mut rng = StdRng::from_rng(OsRng).unwrap();
		// let (threshold, node_num) = (1, 4);

		// // Generate individual key pairs for encryption. These are not suitable for threshold schemes.
		// let sec_keys: Vec<SecretKey> = (0..node_num).map(|_| SecretKey::random()).collect();
		// let pub_keys: BTreeMap<usize, PublicKey> = sec_keys
		// 	.iter()
		// 	.map(SecretKey::public_key)
		// 	.enumerate()
		// 	.collect();

		// let sk = sec_keys[0].clone();
		// let skg =
		// 	SyncKeyGen::new(0, sk, pub_keys.clone(), threshold, &mut rand::thread_rng()).unwrap();
		// let kgm = KeyGenMessage::Part(skg.1.unwrap());
		// let encoded: Vec<u8> = kgm.encode();
		// let decoded = KeyGenMessage::decode(&mut encoded.as_slice()).unwrap();
		// assert_eq!(kgm, decoded);
	}
}
