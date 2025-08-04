use core::mem::size_of;
use pinocchio::{
    ProgramResult, account_info::AccountInfo, program_error::ProgramError,
    pubkey::find_program_address,
};
extern crate alloc;
use alloc::string::{String, ToString};


// Struct die accounts groepeert
pub struct SendProofAccounts<'a> {
    pub payer: &'a AccountInfo,
    pub proof: &'a AccountInfo,
    pub system_program: &'a AccountInfo,
}

// Implementatie om van slice van accounts een struct te maken
impl<'a> TryFrom<&'a [AccountInfo]> for SendProofAccounts<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a [AccountInfo]) -> Result<Self, Self::Error> {
        let [payer, proof, system_program] = accounts else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // TODO: add account checks

        Ok(Self {
            payer: payer,
            proof: proof,
            system_program: system_program,
        })
    }
}
pub struct ProofInstructionData {
    pub seed: u64,
    pub data: String,
}

impl<'a> TryFrom<&'a [u8]> for ProofInstructionData {
    type Error = ProgramError;

    fn try_from(data: &'a [u8]) -> Result<Self, Self::Error> {
        // Check if we at least have 8 bytes for the seed
        if data.len() < size_of::<u64>() {
            return Err(ProgramError::InvalidInstructionData);
        }

        // Extract seed
        let seed = u64::from_le_bytes(data[0..8].try_into().unwrap());

        // Extract remaining bytes as UTF-8 string
        let string_data = core::str::from_utf8(&data[8..])
            .map_err(|_| ProgramError::InvalidInstructionData)?;

        // Optional: Reject empty strings
        if string_data.is_empty() {
            return Err(ProgramError::InvalidInstructionData);
        }

        Ok(ProofInstructionData {
            seed,
            data: string_data.to_string(),
        })
    }
}