use anyhow::Result;
use tracing::error;
use turborepo_ui::GREY;

use crate::commands::CommandBase;

pub fn logout(base: &mut CommandBase) -> Result<()> {
    if let Err(err) = base.user_config_mut()?.set_token(None) {
        error!("could not logout. Something went wrong: {}", err);
        return Err(err.into());
    }

    println!("{}", base.ui.apply(GREY.apply_to(">>> Logged out")));
    Ok(())
}