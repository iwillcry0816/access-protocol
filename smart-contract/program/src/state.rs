use crate::error::AccessError;
use bonfida_utils::BorshSize;
use borsh::{BorshDeserialize, BorshSerialize};
use bytemuck::{from_bytes_mut, try_cast_slice_mut, Pod, Zeroable};
use solana_program::account_info::AccountInfo;
use solana_program::clock::Clock;
use solana_program::entrypoint::ProgramResult;
use solana_program::msg;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::Sysvar;
use std::cell::RefMut;
use std::mem::size_of;

// Just a random mint for now
pub const MEDIA_MINT: Pubkey =
    solana_program::pubkey!("EchesyfXePKdLtoiZSL8pBe8Myagyy8ZRqsACNCFGnvp");

pub const SECONDS_IN_DAY: u64 = 3600 * 24;

pub const STAKER_MULTIPLIER: u64 = 80;
pub const OWNER_MULTIPLIER: u64 = 100 - STAKER_MULTIPLIER;
pub const STAKE_BUFFER_LEN: u64 = 365;

#[derive(BorshSerialize, BorshDeserialize, BorshSize, PartialEq)]
pub enum Tag {
    Uninitialized,
    StakePool,
    StakeAccount,
    // Bond accounts are inactive until the buyer transfered the funds
    InactiveBondAccount,
    BondAccount,
    CentralState,
    Deleted,
}

#[derive(BorshSerialize, BorshDeserialize, BorshSize, Copy, Clone, Pod, Zeroable)]
#[repr(C)]
pub struct StakePoolHeader {
    // Tag
    pub tag: u8,

    // Stake pool nonce
    pub nonce: u8,

    // Updated by a trustless cranker
    pub current_day_idx: u16,

    // Padding
    pub _padding: [u8; 4],

    // Minimum amount to stake to get access to the pool
    pub minimum_stake_amount: u64,

    // Total amount staked in the pool
    pub total_staked: u64,

    // Last unix timestamp when rewards were paid to the pool owner
    // through a permissionless crank
    pub last_crank_time: i64,

    // Last time the stake pool owner claimed
    pub last_claimed_time: i64,

    // Owner of the stake pool
    pub owner: [u8; 32],

    // Address to which rewards are sent
    pub rewards_destination: [u8; 32],

    // Stake pool vault
    pub vault: [u8; 32],
}

pub struct StakePool<'a> {
    pub header: RefMut<'a, StakePoolHeader>,
    pub balances: RefMut<'a, [u128]>, // of length STAKE_BUFFER_LEN
}

impl<'a> StakePool<'a> {
    pub fn get_checked<'b: 'a>(account_info: &'a AccountInfo<'b>) -> Result<Self, ProgramError> {
        let (header, balances) = RefMut::map_split(account_info.data.borrow_mut(), |s| {
            let (hd, rem) = s.split_at_mut(size_of::<StakePoolHeader>());
            (
                from_bytes_mut::<StakePoolHeader>(hd),
                try_cast_slice_mut(rem).unwrap(),
            )
        });

        if header.tag != Tag::StakePool as u8 && header.tag != Tag::Uninitialized as u8 {
            return Err(AccessError::DataTypeMismatch.into());
        }

        Ok(StakePool { header, balances })
    }

    pub fn push_balances_buff(&mut self, val: u128) {
        self.balances[((self.header.current_day_idx as u64) % STAKE_BUFFER_LEN) as usize] = val;
        self.header.current_day_idx += 1;
    }

    pub fn create_key(
        nonce: &u8,
        owner: &Pubkey,
        destination: &Pubkey,
        program_id: &Pubkey,
    ) -> Pubkey {
        let seeds: &[&[u8]] = &[
            StakePoolHeader::SEED.as_bytes(),
            &owner.to_bytes(),
            &destination.to_bytes(),
            &[*nonce],
        ];
        Pubkey::create_program_address(seeds, program_id).unwrap()
    }
}

impl StakePoolHeader {
    pub const SEED: &'static str = "stake_pool";

    pub fn new(
        owner: Pubkey,
        rewards_destination: Pubkey,
        nonce: u8,
        vault: Pubkey,
        minimum_stake_amount: u64,
    ) -> Self {
        Self {
            tag: Tag::StakePool as u8,
            total_staked: 0,
            current_day_idx: 0,
            _padding: [0; 4],
            last_crank_time: Clock::get().unwrap().unix_timestamp,
            last_claimed_time: Clock::get().unwrap().unix_timestamp,
            owner: owner.to_bytes(),
            rewards_destination: rewards_destination.to_bytes(),
            nonce,
            vault: vault.to_bytes(),
            minimum_stake_amount,
        }
    }

    // pub fn save(&self, mut dst: &mut [u8]) {
    //     self.serialize(&mut dst).unwrap()
    // }

    // pub fn from_account_info(a: &AccountInfo) -> Result<StakePool, ProgramError> {
    //     let mut data = &a.data.borrow() as &[u8];
    //     if data[0] != Tag::StakePool as u8 && data[0] != Tag::Uninitialized as u8 {
    //         return Err(AccessError::DataTypeMismatch.into());
    //     }
    //     let result = StakePool::deserialize(&mut data)?;
    //     Ok(result)
    // }

    pub fn close(&mut self) {
        self.tag = Tag::Deleted as u8
    }

    pub fn deposit(&mut self, amount: u64) -> ProgramResult {
        self.total_staked = self.total_staked.checked_add(amount).unwrap();
        Ok(())
    }

    pub fn withdraw(&mut self, amount: u64) -> ProgramResult {
        self.total_staked = self.total_staked.checked_sub(amount).unwrap();
        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, BorshSize)]
pub struct StakeAccount {
    // Tag
    pub tag: Tag,

    // Owner of the stake account
    pub owner: Pubkey,

    // Amount staked in the account
    pub stake_amount: u64,

    // Stake pool to which the account belongs to
    pub stake_pool: Pubkey,

    // Last unix timestamp where rewards were claimed
    pub last_claimed_time: i64,

    // Minimum stakeable amount of the pool when the account
    // was created
    pub pool_minimum_at_creation: u64,
}

impl StakeAccount {
    pub const SEED: &'static str = "stake_account";

    pub fn new(
        owner: Pubkey,
        stake_pool: Pubkey,
        current_time: i64,
        pool_minimum_at_creation: u64,
    ) -> Self {
        Self {
            tag: Tag::StakeAccount,
            owner,
            stake_amount: 0,
            stake_pool,
            last_claimed_time: current_time,
            pool_minimum_at_creation,
        }
    }

    pub fn create_key(
        nonce: &u8,
        owner: &Pubkey,
        stake_pool: &Pubkey,
        program_id: &Pubkey,
    ) -> Pubkey {
        let seeds: &[&[u8]] = &[
            StakeAccount::SEED.as_bytes(),
            &owner.to_bytes(),
            &stake_pool.to_bytes(),
            &[*nonce],
        ];
        Pubkey::create_program_address(seeds, program_id).unwrap()
    }

    pub fn save(&self, mut dst: &mut [u8]) {
        self.serialize(&mut dst).unwrap()
    }

    pub fn from_account_info(a: &AccountInfo) -> Result<StakeAccount, ProgramError> {
        let mut data = &a.data.borrow() as &[u8];
        if data[0] != Tag::StakeAccount as u8 && data[0] != Tag::Uninitialized as u8 {
            return Err(AccessError::DataTypeMismatch.into());
        }
        let result = StakeAccount::deserialize(&mut data)?;
        Ok(result)
    }

    pub fn close(&mut self) {
        self.tag = Tag::Deleted
    }

    pub fn deposit(&mut self, amount: u64) -> ProgramResult {
        self.stake_amount = self.stake_amount.checked_add(amount).unwrap();
        Ok(())
    }

    pub fn withdraw(&mut self, amount: u64) -> ProgramResult {
        self.stake_amount = self.stake_amount.checked_sub(amount).unwrap();
        Ok(())
    }
}
#[derive(BorshSerialize, BorshDeserialize, BorshSize)]
pub struct CentralState {
    // Tag
    pub tag: Tag,

    // Central state nonce
    pub signer_nonce: u8,

    // Daily inflation in token amount, inflation is paid from
    // the reserve owned by the central state
    pub daily_inflation: u64,

    // Mint of the token being emitted
    pub token_mint: Pubkey,

    // Authority
    // The public key that can change the inflation
    pub authority: Pubkey,
}

impl CentralState {
    pub fn new(
        signer_nonce: u8,
        daily_inflation: u64,
        token_mint: Pubkey,
        authority: Pubkey,
    ) -> Self {
        Self {
            tag: Tag::CentralState,
            signer_nonce,
            daily_inflation,
            token_mint,
            authority,
        }
    }

    pub fn create_key(signer_nonce: &u8, program_id: &Pubkey) -> Pubkey {
        let signer_seeds: &[&[u8]] = &[&program_id.to_bytes(), &[*signer_nonce]];
        Pubkey::create_program_address(signer_seeds, program_id).unwrap()
    }

    pub fn find_key(program_id: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[&program_id.to_bytes()], program_id)
    }

    pub fn save(&self, mut dst: &mut [u8]) {
        self.serialize(&mut dst).unwrap()
    }

    pub fn from_account_info(a: &AccountInfo) -> Result<CentralState, ProgramError> {
        let mut data = &a.data.borrow() as &[u8];
        if data[0] != Tag::CentralState as u8 && data[0] != Tag::Uninitialized as u8 {
            return Err(AccessError::DataTypeMismatch.into());
        }
        let result = CentralState::deserialize(&mut data)?;
        Ok(result)
    }
}

pub const BOND_SIGNER_THRESHOLD: u64 = 1;
pub const AUTHORIZED_BOND_SELLERS: [Pubkey; 1] = [solana_program::pubkey!(
    "ERNVcTG8sGynQjy6BKr3qotMusv3Zo1pJsbGdBgy9eQQ"
)];

#[derive(BorshSerialize, BorshDeserialize, BorshSize)]
pub struct BondAccount {
    // Tag
    pub tag: Tag,

    // Owner of the bond
    pub owner: Pubkey,

    // Total amount sold
    pub total_amount_sold: u64,

    // Total staked tokens
    pub total_staked: u64,

    // Total quote token
    pub total_quote_amount: u64,

    // Quote mint used to buy the bond
    pub quote_mint: Pubkey,

    // Seller token account (i.e destination of the quote tokens)
    pub seller_token_account: Pubkey,

    // Unlock start date
    pub unlock_start_date: i64,

    // Unlock period
    // time interval at which the tokens unlock
    pub unlock_period: i64,

    // Unlock amount
    // amount unlocked at every unlock_period
    pub unlock_amount: u64,

    // Last unlock date
    pub last_unlock_time: i64,

    // Total amount unlocked (metric)
    pub total_unlocked_amount: u64,

    // Minimum stakeable amount of the pool when the account
    // was created
    pub pool_minimum_at_creation: u64,

    // Stake pool to which the account belongs to
    pub stake_pool: Pubkey,

    // Last unix timestamp where rewards were claimed
    pub last_claimed_time: i64,

    // Sellers who signed for the sell of the bond account
    pub sellers: Vec<Pubkey>,
}

impl BondAccount {
    pub const SEED: &'static str = "bond_account";

    pub fn create_key(owner: &Pubkey, total_amount_sold: u64, program_id: &Pubkey) -> (Pubkey, u8) {
        let seeds: &[&[u8]] = &[
            BondAccount::SEED.as_bytes(),
            &owner.to_bytes(),
            &total_amount_sold.to_be_bytes(),
        ];
        Pubkey::find_program_address(seeds, program_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner: Pubkey,
        total_amount_sold: u64,
        total_quote_amount: u64,
        quote_mint: Pubkey,
        seller_token_account: Pubkey,
        unlock_start_date: i64,
        unlock_period: i64,
        unlock_amount: u64,
        last_unlock_time: i64,
        pool_minimum_at_creation: u64,
        stake_pool: Pubkey,
        last_claimed_time: i64,
        seller: Pubkey,
    ) -> Self {
        let sellers = vec![seller];
        Self {
            tag: Tag::InactiveBondAccount,
            owner,
            total_amount_sold,
            total_staked: total_amount_sold,
            total_quote_amount,
            quote_mint,
            seller_token_account,
            unlock_start_date,
            unlock_period,
            unlock_amount,
            last_unlock_time,
            total_unlocked_amount: 0,
            stake_pool,
            last_claimed_time,
            sellers,
            pool_minimum_at_creation,
        }
    }

    pub fn save(&self, mut dst: &mut [u8]) {
        self.serialize(&mut dst).unwrap()
    }

    pub fn is_active(&self) -> bool {
        self.tag == Tag::BondAccount
    }

    pub fn activate(&mut self) {
        self.tag = Tag::BondAccount
    }

    pub fn from_account_info(
        a: &AccountInfo,
        allow_inactive: bool,
    ) -> Result<BondAccount, ProgramError> {
        let mut data = &a.data.borrow() as &[u8];
        let tag = if allow_inactive {
            Tag::InactiveBondAccount
        } else {
            Tag::BondAccount
        };
        if data[0] != tag as u8 && data[0] != Tag::Uninitialized as u8 {
            return Err(AccessError::DataTypeMismatch.into());
        }
        let result = BondAccount::deserialize(&mut data)?;
        Ok(result)
    }

    pub fn calc_unlock_amount(&self, missed_periods: u64) -> Result<u64, ProgramError> {
        msg!("{}", missed_periods);
        let unlock_amount = missed_periods * self.unlock_amount;
        msg!(
            "unlock amount {} total amount {}",
            unlock_amount,
            self.total_amount_sold
        );
        if self
            .total_unlocked_amount
            .checked_add(unlock_amount)
            .ok_or(AccessError::Overflow)?
            > self.total_amount_sold
        {
            Ok(self
                .total_amount_sold
                .checked_sub(unlock_amount)
                .ok_or(AccessError::Overflow)?)
        } else {
            Ok(unlock_amount)
        }
    }
}
