use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_program,
};

use crate::utils::{assert_empty_stake_pool, check_account_key, check_account_owner, check_signer};
use bonfida_utils::{BorshSize, InstructionsAccount};

use crate::error::MediaError;
use crate::state::StakePool;

#[derive(BorshDeserialize, BorshSerialize, BorshSize)]
pub struct Params {
    // The PDA nonce
    pub nonce: u8,
    // Name of the stake pool
    pub name: String,
    // Destination of the rewards
    pub destination: [u8; 32],
}

#[derive(InstructionsAccount)]
struct Accounts<'a, T> {
    #[cons(writable)]
    stake_pool_account: &'a T,
    system_program: &'a T,
    #[cons(writable, signer)]
    owner: &'a T,
}

impl<'a, 'b: 'a> Accounts<'a, AccountInfo<'b>> {
    pub fn parse(
        accounts: &'a [AccountInfo<'b>],
        program_id: &Pubkey,
    ) -> Result<Self, ProgramError> {
        let accounts_iter = &mut accounts.iter();
        let accounts = Accounts {
            stake_pool_account: next_account_info(accounts_iter)?,
            system_program: next_account_info(accounts_iter)?,
            owner: next_account_info(accounts_iter)?,
        };

        // Check keys
        check_account_key(
            accounts.system_program,
            &system_program::ID,
            MediaError::WrongSystemProgram,
        )?;

        // Check ownership
        check_account_owner(
            accounts.stake_pool_account,
            program_id,
            MediaError::WrongOwner,
        )?;

        // Check signer
        check_signer(accounts.owner, MediaError::StakePoolOwnerMustSign)?;

        Ok(accounts)
    }
}

pub fn process_close_stake_pool(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    params: Params,
) -> ProgramResult {
    let accounts = Accounts::parse(accounts, program_id)?;

    let Params {
        nonce,
        name,
        destination,
    } = params;

    let derived_stake_pool_key = StakePool::create_key(
        &nonce,
        &name,
        &accounts.owner.key.to_bytes(),
        &destination,
        program_id,
    );

    check_account_key(
        accounts.stake_pool_account,
        &derived_stake_pool_key,
        MediaError::AccountNotDeterministic,
    )?;

    let mut stake_pool = StakePool::from_account_info(accounts.stake_pool_account).unwrap();

    check_account_key(
        accounts.owner,
        &Pubkey::new(&stake_pool.owner),
        MediaError::WrongStakePoolOwner,
    )?;

    assert_empty_stake_pool(&stake_pool)?;

    stake_pool.close();
    stake_pool.save(&mut accounts.stake_pool_account.data.borrow_mut());

    Ok(())
}
