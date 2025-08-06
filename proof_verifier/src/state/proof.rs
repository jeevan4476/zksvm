use core::mem::size_of;
use pinocchio::{
    program_error::ProgramError,
    pubkey::Pubkey,
};

// replicate C-like memory layout
#[repr(C)]
pub struct Proof {
    pub seed: u64,      // Random seed for PDA derivation
    pub maker: Pubkey,  // Creator of the escrow
    pub data: String,
    pub bump: [u8; 1],  // PDA bump seed
}