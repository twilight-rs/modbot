use futures_util::stream::StreamExt;
use std::{env, error::Error};
use twilight_gateway::{Event, Intents, Shard};
use twilight_http::Client;
use twilight_model::id::{GuildId, RoleId};

/// 5 different roles which we like to set to colors based on the season
/// in North America.
const COLOR_ROLES: [u64; 5] = [
    787463754823630871,
    787463294419992606,
    787463761102897153,
    786293768366063686,
    787463763540312097,
];

/// Where the bot actually takes place.
async fn run() -> Result<(), Box<dyn Error>> {
    // Initialize the tracing subscriber.
    tracing_subscriber::fmt::init();

    // ID of the base role that everyone in the Twilight guild should have.
    fn twilight_base_role() -> RoleId {
        RoleId::new(745812265005219894).expect("non zero")
    }

    // ID of the Twilight support and development guild.
    fn twilight_guild_id() -> GuildId {
        GuildId::new(745809834183753828).expect("non zero")
    }

    // Get the token from the environment.
    let token = env::var("DISCORD_TOKEN")?;

    // Initialize the HTTP client.
    let http = Client::new(token.clone());

    // Since this bot should only be in one guild, initialize and start
    // up only one shard.
    let (shard, mut events) = Shard::new(token, Intents::GUILDS | Intents::GUILD_MEMBERS);
    shard.start().await?;

    // Process events as they come in.
    //
    // This uses the [`StreamExt::next`] method as a convenience.
    //
    // [`StreamExt::next`]: futures_util::stream::StreamExt::next
    while let Some(event) = events.next().await {
        match event {
            // Process new members.
            Event::MemberAdd(member_add) if member_add.guild_id == twilight_guild_id() => {
                let user_id = member_add.user.id;

                // Add the base twilight role to all new members.
                http.add_guild_member_role(twilight_guild_id(), user_id, twilight_base_role())
                    .exec()
                    .await?;

                // Choose a color role based off of the user's ID.
                let choice = COLOR_ROLES[(user_id.get() % 5) as usize];
                http.add_guild_member_role(
                    twilight_guild_id(),
                    user_id,
                    RoleId::new(choice).expect("non zero"),
                )
                .exec()
                .await?;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Entry point to the bot.
///
/// It starts up the `run` function, catches and reports any error that
/// might have happened, and exits.
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    if let Err(source) = run().await {
        eprintln!("{}", source);
    }

    Ok(())
}
