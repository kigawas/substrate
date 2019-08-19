use std::{
	collections::VecDeque,
	fmt::Debug,
	marker::PhantomData,
	sync::Arc,
	time::{Duration, Instant},
};

use client::blockchain::HeaderBackend;
use client::{
	backend::{AuxStore, Backend},
	error::Error as ClientError,
	error::Result as ClientResult,
	BlockchainEvents, CallExecutor, Client,
};
use codec::{Decode, Encode};
use consensus_common::SelectChain;
use futures::{future::Loop as FutureLoop, prelude::*, stream::Fuse, sync::mpsc};
use hbbft::crypto::{PublicKey, SecretKey, SignatureShare};
use hbbft_primitives::HbbftApi;
use inherents::InherentDataProviders;
use log::{debug, error, info, warn};
use network;
use runtime_primitives::generic::BlockId;
use runtime_primitives::traits::{Block as BlockT, DigestFor, NumberFor, ProvideRuntimeApi};

const INDEX_KEY: &[u8] = b"multi_ecdsa_index";

type Index = u16;

#[derive(Debug)]
pub struct SharedState {
	pub current_index: Index,
}

pub(crate) fn load_decode<B: AuxStore, T: Decode>(
	backend: &B,
	key: &[u8],
) -> ClientResult<Option<T>> {
	match backend.get_aux(key)? {
		None => Ok(None),
		Some(t) => T::decode(&mut &t[..])
			.map_err(|e| ClientError::Backend(format!("GRANDPA DB is corrupted: {}", e.what())))
			.map(Some),
	}
}

pub(crate) fn load_persistent<B>(backend: &B) -> ClientResult<SharedState>
where
	B: AuxStore,
{
	let index: Option<Index> = load_decode(backend, INDEX_KEY)?;
	match index {
		Some(index) => {
			let current_index: Index = index;
			Ok(SharedState { current_index })
		}
		_ => {
			let index: Index = 0;
			backend.insert_aux(&[(INDEX_KEY, Encode::encode(&index).as_slice())], &[]);
			Ok(SharedState {
				current_index: index,
			})
		}
	}
}

pub(crate) fn set_index<B: AuxStore>(backend: &B, index: Index) -> ClientResult<()> {
	backend.insert_aux(&[(INDEX_KEY, Encode::encode(&index).as_slice())], &[])
}
