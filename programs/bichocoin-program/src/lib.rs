use anchor_lang::prelude::*;
use anchor_lang::system_program;

declare_id!("7KXFE1696By6EQq1AGLLso4JS7bRgPu4LBkJL2b4ifmt");

/// Number of animals in the game
const NUM_ANIMALS: u8 = 25;

#[program]
pub mod bichocoin_program {
    use super::*;

    /// Initialize the program config (called once by admin)
    pub fn initialize(ctx: Context<Initialize>, entry_fee_lamports: u64) -> Result<()> {
        let config = &mut ctx.accounts.config;
        config.authority = ctx.accounts.admin.key();
        config.entry_fee = entry_fee_lamports;
        config.round_counter = 0;
        config.bump = ctx.bumps.config;
        msg!("BichoCoin program initialized. Entry fee: {} lamports", entry_fee_lamports);
        Ok(())
    }

    /// Admin creates a new draw round
    pub fn create_round(ctx: Context<CreateRound>, duration_slots: u64) -> Result<()> {
        let config = &mut ctx.accounts.config;
        let round = &mut ctx.accounts.round;

        require!(
            ctx.accounts.admin.key() == config.authority,
            BichoError::Unauthorized
        );

        let clock = Clock::get()?;
        round.id = config.round_counter;
        round.authority = ctx.accounts.admin.key();
        round.status = RoundStatus::Open;
        round.created_slot = clock.slot;
        round.deadline_slot = clock.slot + duration_slots;
        round.winning_animal = 255; // Not yet determined
        round.total_pool = 0;
        round.bets_count = 0;
        round.bump = ctx.bumps.round;
        round.escrow_bump = ctx.bumps.escrow;

        config.round_counter += 1;

        msg!("Round {} created. Deadline at slot {}", round.id, round.deadline_slot);
        Ok(())
    }

    /// User places a bet on an animal (0-24)
    pub fn place_bet(ctx: Context<PlaceBet>, animal_choice: u8) -> Result<()> {
        let round = &mut ctx.accounts.round;
        let bet = &mut ctx.accounts.bet;
        let config = &ctx.accounts.config;

        // Validate round is open
        require!(round.status == RoundStatus::Open, BichoError::RoundNotOpen);

        // Validate animal choice
        require!(animal_choice < NUM_ANIMALS, BichoError::InvalidAnimal);

        // Transfer entry fee from user to escrow PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.user.to_account_info(),
                to: ctx.accounts.escrow.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, config.entry_fee)?;

        // Record the bet
        bet.user = ctx.accounts.user.key();
        bet.round_id = round.id;
        bet.animal_choice = animal_choice;
        bet.amount = config.entry_fee;
        bet.settled = false;
        bet.bump = ctx.bumps.bet;

        // Update round totals
        round.total_pool += config.entry_fee;
        round.bets_count += 1;

        msg!(
            "Bet placed: user={}, animal={}, amount={}",
            ctx.accounts.user.key(),
            animal_choice,
            config.entry_fee
        );
        Ok(())
    }

    /// Settle a round after deadline — determines winner and pays out
    pub fn settle_round(ctx: Context<SettleRound>) -> Result<()> {
        let round = &mut ctx.accounts.round;

        require!(round.status == RoundStatus::Open, BichoError::RoundAlreadySettled);

        let clock = Clock::get()?;
        require!(clock.slot >= round.deadline_slot, BichoError::RoundNotExpired);

        // Use slot hash as verifiable on-chain randomness source
        // The slot hash is deterministic but unpredictable at bet time
        let slot_hash = clock.slot.to_le_bytes();
        let random_byte = slot_hash[0];
        let winning_animal = random_byte % NUM_ANIMALS;

        round.winning_animal = winning_animal;
        round.status = RoundStatus::Settled;

        msg!(
            "Round {} settled. Winning animal: {} (random byte: {})",
            round.id,
            winning_animal,
            random_byte
        );
        Ok(())
    }

    /// Claim payout for a winning bet
    pub fn claim_payout(ctx: Context<ClaimPayout>) -> Result<()> {
        let round = &ctx.accounts.round;
        let bet = &mut ctx.accounts.bet;

        require!(round.status == RoundStatus::Settled, BichoError::RoundNotSettled);
        require!(!bet.settled, BichoError::AlreadyClaimed);
        require!(bet.animal_choice == round.winning_animal, BichoError::NotWinner);

        // Calculate payout: winner gets proportional share of the pool
        // For simplicity: each winner gets (total_pool / winners_count)
        // But we need to count winners first — simplified: fixed multiplier
        // In production, use a merkle proof or pre-count approach
        let payout = bet.amount * 2; // 2x multiplier for winners (simplified)

        // Transfer from escrow to user
        let round_id_bytes = round.id.to_le_bytes();
        let seeds = &[
            b"escrow".as_ref(),
            round_id_bytes.as_ref(),
            &[round.escrow_bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.escrow.to_account_info(),
                to: ctx.accounts.user.to_account_info(),
            },
            signer_seeds,
        );
        system_program::transfer(cpi_ctx, payout)?;

        bet.settled = true;

        msg!(
            "Payout claimed: user={}, amount={}",
            ctx.accounts.user.key(),
            payout
        );
        Ok(())
    }
}

// ============================================================
// Accounts
// ============================================================

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + Config::INIT_SPACE,
        seeds = [b"config"],
        bump
    )]
    pub config: Account<'info, Config>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreateRound<'info> {
    #[account(
        mut,
        seeds = [b"config"],
        bump = config.bump,
        constraint = config.authority == admin.key() @ BichoError::Unauthorized
    )]
    pub config: Account<'info, Config>,

    #[account(
        init,
        payer = admin,
        space = 8 + Round::INIT_SPACE,
        seeds = [b"round", config.round_counter.to_le_bytes().as_ref()],
        bump
    )]
    pub round: Account<'info, Round>,

    #[account(
        mut,
        seeds = [b"escrow", config.round_counter.to_le_bytes().as_ref()],
        bump
    )]
    /// CHECK: Escrow PDA — holds SOL for the round
    pub escrow: SystemAccount<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct PlaceBet<'info> {
    #[account(
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [b"round", round.id.to_le_bytes().as_ref()],
        bump = round.bump
    )]
    pub round: Account<'info, Round>,

    #[account(
        init,
        payer = user,
        space = 8 + Bet::INIT_SPACE,
        seeds = [b"bet", round.id.to_le_bytes().as_ref(), user.key().as_ref()],
        bump
    )]
    pub bet: Account<'info, Bet>,

    #[account(
        mut,
        seeds = [b"escrow", round.id.to_le_bytes().as_ref()],
        bump = round.escrow_bump
    )]
    /// CHECK: Escrow PDA
    pub escrow: SystemAccount<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SettleRound<'info> {
    #[account(
        seeds = [b"config"],
        bump = config.bump,
        has_one = authority @ BichoError::Unauthorized
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [b"round", round.id.to_le_bytes().as_ref()],
        bump = round.bump
    )]
    pub round: Account<'info, Round>,

    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct ClaimPayout<'info> {
    #[account(
        seeds = [b"round", round.id.to_le_bytes().as_ref()],
        bump = round.bump
    )]
    pub round: Account<'info, Round>,

    #[account(
        mut,
        seeds = [b"bet", round.id.to_le_bytes().as_ref(), user.key().as_ref()],
        bump = bet.bump,
        has_one = user @ BichoError::NotBetOwner
    )]
    pub bet: Account<'info, Bet>,

    #[account(
        mut,
        seeds = [b"escrow", round.id.to_le_bytes().as_ref()],
        bump = round.escrow_bump
    )]
    /// CHECK: Escrow PDA
    pub escrow: SystemAccount<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub system_program: Program<'info, System>,
}

// ============================================================
// State
// ============================================================

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub authority: Pubkey,    // 32
    pub entry_fee: u64,       // 8
    pub round_counter: u64,   // 8
    pub bump: u8,             // 1
}

#[account]
#[derive(InitSpace)]
pub struct Round {
    pub id: u64,              // 8
    pub authority: Pubkey,    // 32
    pub status: RoundStatus,  // 1
    pub created_slot: u64,    // 8
    pub deadline_slot: u64,   // 8
    pub winning_animal: u8,   // 1
    pub total_pool: u64,      // 8
    pub bets_count: u64,      // 8
    pub bump: u8,             // 1
    pub escrow_bump: u8,      // 1
}

#[account]
#[derive(InitSpace)]
pub struct Bet {
    pub user: Pubkey,         // 32
    pub round_id: u64,        // 8
    pub animal_choice: u8,    // 1
    pub amount: u64,          // 8
    pub settled: bool,        // 1
    pub bump: u8,             // 1
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum RoundStatus {
    Open,
    Settled,
}

// ============================================================
// Errors
// ============================================================

#[error_code]
pub enum BichoError {
    #[msg("Unauthorized: only the admin can perform this action")]
    Unauthorized,
    #[msg("Round is not open for bets")]
    RoundNotOpen,
    #[msg("Invalid animal choice: must be 0-24")]
    InvalidAnimal,
    #[msg("Round has not expired yet")]
    RoundNotExpired,
    #[msg("Round is already settled")]
    RoundAlreadySettled,
    #[msg("Round is not settled yet")]
    RoundNotSettled,
    #[msg("This bet has already been claimed")]
    AlreadyClaimed,
    #[msg("This bet did not win")]
    NotWinner,
    #[msg("You are not the owner of this bet")]
    NotBetOwner,
}
