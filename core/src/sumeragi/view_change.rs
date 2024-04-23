//! Structures related to proofs and reasons of view changes.
//! Where view change is a process of changing topology due to some faulty network behavior.

use derive_more::{Deref, DerefMut};
use eyre::Result;
use iroha_crypto::{HashOf, PrivateKey, SignatureOf};
use iroha_data_model::block::SignedBlock;
use parity_scale_codec::{Decode, Encode};
use thiserror::Error;

use super::network_topology::Topology;

type ViewChangeProofSignature = (u64, SignatureOf<ProofPayload>);

/// Error emerge during insertion of `Proof` into `ProofChain`
#[derive(Error, displaydoc::Display, Debug, Clone, Copy)]
#[allow(missing_docs)]
pub enum Error {
    /// Block hash of proof doesn't match hash of proof chain
    BlockHashMismatch,
    /// View change index is not present in proof chain
    ViewChangeNotFound,
}

#[derive(Debug, Clone, Decode, Encode)]
struct ProofPayload {
    /// Hash of the latest committed block.
    latest_block_hash: Option<HashOf<SignedBlock>>,
    /// Within a round, what is the index of the view change this proof is trying to prove.
    view_change_index: u64,
}

/// The proof of a view change. It needs to be signed by f+1 peers for proof to be valid and view change to happen.
#[derive(Debug, Clone, Decode, Encode)]
pub struct SignedProof {
    signatures: Vec<ViewChangeProofSignature>,
    /// Collection of signatures from the different peers.
    payload: ProofPayload,
}

/// Builder for proofs
#[repr(transparent)]
pub struct ProofBuilder(SignedProof);

impl ProofBuilder {
    /// Constructor from index.
    pub fn new(latest_block_hash: Option<HashOf<SignedBlock>>, view_change_index: u64) -> Self {
        let proof = SignedProof {
            payload: ProofPayload {
                latest_block_hash,
                view_change_index,
            },
            signatures: [].into_iter().collect(),
        };

        Self(proof)
    }

    /// Sign this message with the peer's public and private key.
    pub fn sign(mut self, node_pos: u64, private_key: &PrivateKey) -> SignedProof {
        let signature = SignatureOf::new(private_key, &self.0.payload);
        self.0.signatures.push((node_pos, signature));
        self.0
    }
}

impl SignedProof {
    /// Verify the signatures of `other` and add them to this proof.
    fn merge_signatures(&mut self, other: Vec<ViewChangeProofSignature>, topology: &Topology) {
        for (node_pos, signature) in other {
            let public_key = topology.as_ref()[node_pos as usize].public_key();

            if signature.verify(public_key, &self.payload).is_ok() {
                self.signatures.push((node_pos, signature));
            }
        }
    }

    /// Verify if the proof is valid, given the peers in `topology`.
    fn verify(&self, topology: &Topology) -> bool {
        let valid_count = self
            .signatures
            .iter()
            .filter(|&(node_pos, signature)| {
                let public_key = topology.as_ref()[*node_pos as usize].public_key();
                signature.verify(public_key, &self.payload).is_ok()
            })
            .count();

        // See Whitepaper for the information on this limit.
        #[allow(clippy::int_plus_one)]
        {
            valid_count >= topology.max_faults() + 1
        }
    }
}

/// Structure representing sequence of view change proofs.
#[derive(Debug, Clone, Encode, Decode, Deref, DerefMut, Default)]
pub struct ProofChain(Vec<SignedProof>);

impl ProofChain {
    /// Verify the view change proof chain.
    pub fn verify_with_state(
        &self,
        topology: &Topology,
        latest_block_hash: Option<HashOf<SignedBlock>>,
    ) -> usize {
        self.iter()
            .enumerate()
            .take_while(|(i, proof)| {
                proof.payload.latest_block_hash == latest_block_hash
                    && proof.payload.view_change_index == (*i as u64)
                    && proof.verify(topology)
            })
            .count()
    }

    /// Remove invalid proofs from the chain.
    pub fn prune(&mut self, latest_block_hash: Option<HashOf<SignedBlock>>) {
        let valid_count = self
            .iter()
            .enumerate()
            .take_while(|(i, proof)| {
                proof.payload.latest_block_hash == latest_block_hash
                    && proof.payload.view_change_index == (*i as u64)
            })
            .count();
        self.truncate(valid_count);
    }

    /// Attempt to insert a view chain proof into this `ProofChain`.
    ///
    /// # Errors
    /// - If proof latest block hash doesn't match peer latest block hash
    /// - If proof view change number differs from view change number
    pub fn insert_proof(
        &mut self,
        new_proof: SignedProof,
        topology: &Topology,
        latest_block_hash: Option<HashOf<SignedBlock>>,
    ) -> Result<(), Error> {
        if new_proof.payload.latest_block_hash != latest_block_hash {
            return Err(Error::BlockHashMismatch);
        }
        let next_unfinished_view_change = self.verify_with_state(topology, latest_block_hash);
        if new_proof.payload.view_change_index != (next_unfinished_view_change as u64) {
            return Err(Error::ViewChangeNotFound); // We only care about the current view change that may or may not happen.
        }

        let is_proof_chain_incomplete = next_unfinished_view_change < self.len();
        if is_proof_chain_incomplete {
            self[next_unfinished_view_change].merge_signatures(new_proof.signatures, topology);
        } else {
            self.push(new_proof);
        }
        Ok(())
    }

    /// Add latest proof from other chain into current.
    ///
    /// # Errors
    /// - If there is mismatch between `other` proof chain latest block hash and peer's latest block hash
    /// - If `other` proof chain doesn't have proof for current view chain
    pub fn merge(
        &mut self,
        mut other: Self,
        topology: &Topology,
        latest_block_hash: Option<HashOf<SignedBlock>>,
    ) -> Result<(), Error> {
        // Prune to exclude invalid proofs
        other.prune(latest_block_hash);
        if other.is_empty() {
            return Err(Error::BlockHashMismatch);
        }

        let next_unfinished_view_change = self.verify_with_state(topology, latest_block_hash);
        let is_proof_chain_incomplete = next_unfinished_view_change < self.len();
        let other_contain_additional_proofs = next_unfinished_view_change < other.len();

        match (is_proof_chain_incomplete, other_contain_additional_proofs) {
            // Case 1: proof chain is incomplete and other have corresponding proof.
            (true, true) => {
                let new_proof = other.swap_remove(next_unfinished_view_change);
                self[next_unfinished_view_change].merge_signatures(new_proof.signatures, topology);
            }
            // Case 2: proof chain is complete, but other have additional proof.
            (false, true) => {
                let new_proof = other.swap_remove(next_unfinished_view_change);
                self.push(new_proof);
            }
            // Case 3: proof chain is incomplete, but other doesn't contain corresponding proof.
            // Usually this mean that sender peer is behind receiver peer.
            (true, false) => {
                return Err(Error::ViewChangeNotFound);
            }
            // Case 4: proof chain is complete, but other doesn't have any new peer.
            // This considered normal course of action.
            (false, false) => {}
        }

        Ok(())
    }
}
