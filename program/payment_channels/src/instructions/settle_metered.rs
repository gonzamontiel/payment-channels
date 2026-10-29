use pinocchio::{AccountView, Address, ProgramResult, error::ProgramError};

/// Instruction discriminator byte for `settle_metered`.
pub const DISCRIMINATOR: u8 = 10;

pub struct SettleMeteredAccounts<'a> {
    pub channel: &'a mut AccountView,
    pub instructions_sysvar: &'a mut AccountView,
}

impl<'a> TryFrom<&'a mut [AccountView]> for SettleMeteredAccounts<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a mut [AccountView]) -> Result<Self, Self::Error> {
        let [channel, instructions_sysvar] = accounts else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };
        Ok(Self {
            channel,
            instructions_sysvar,
        })
    }
}

pub fn process(_program_id: &Address, accounts: &mut [AccountView]) -> ProgramResult {
    let _accs = SettleMeteredAccounts::try_from(accounts)?;
    Ok(())
}
