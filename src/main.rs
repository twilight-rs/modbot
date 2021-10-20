use chrono::{TimeZone, Utc};
use futures_util::stream::StreamExt;
use std::{
    collections::HashMap,
    env,
    error::Error,
    fmt::Write,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time;
use twilight_gateway::{Event, EventTypeFlags, Intents, Shard};
use twilight_http::{request::channel::reaction::RequestReactionType, Client};
use twilight_model::{
    channel::ReactionType,
    gateway::payload::ReactionAdd,
    id::{ChannelId, EmojiId, GuildId, MessageId, RoleId, UserId},
    user::User,
};
use twilight_standby::Standby;
use twilight_util::snowflake::Snowflake;

const CHANNEL_ID: ChannelId = ChannelId(745818897843879966);
const EMOJI_FERRIS_DOWN: EmojiId = EmojiId(757329113387237510);
const EMOJI_FERRIS_UP: EmojiId = EmojiId(757327511414636684);
const GENERAL_ID: ChannelId = ChannelId(745815089067851807);
const JOIN_ID: ChannelId = ChannelId(781404765791584268);
const TWILIGHT_ID: GuildId = GuildId(745809834183753828);
const TWILIGHT_ROLE: RoleId = RoleId(745812265005219894);

const COLOR_ROLES: [RoleId; 5] = [
    RoleId(787463754823630871),
    RoleId(787463294419992606),
    RoleId(787463761102897153),
    RoleId(786293768366063686),
    RoleId(787463763540312097),
];

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Recent {
    discrim: String,
    id: UserId,
    name: String,
}

struct MemberState {
    recent: HashMap<UserId, Recent>,
    triggered: Option<MessageId>,
}

struct State {
    http: Client,
    standby: Standby,
}

enum TriggerStatus {
    Triggered,
    ShouldTrigger,
    Inactive,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

    let token = env::var("DISCORD_TOKEN")?;

    let http = Client::new(token.clone());
    let mut shard = Shard::new(
        token,
        Intents::GUILD_MESSAGE_REACTIONS | Intents::GUILD_MEMBERS,
    );
    let mut events = shard.some_events(EventTypeFlags::MEMBER_ADD | EventTypeFlags::REACTION_ADD);
    shard.start().await.unwrap();

    let state = Arc::new(State {
        http,
        standby: Standby::new(),
    });

    let members = Arc::new(Mutex::new(MemberState {
        recent: HashMap::new(),
        triggered: None,
    }));

    while let Some(event) = events.next().await {
        state.standby.process(&event);

        match event {
            Event::MemberAdd(event) if event.guild_id == TWILIGHT_ID => {
                let id = event.0.user.id;

                // just add roles for now
                let _ = state
                    .http
                    .add_guild_member_role(TWILIGHT_ID, id, TWILIGHT_ROLE)
                    .await;
                let choice = id.0 % 5;
                let _ = state
                    .http
                    .add_guild_member_role(TWILIGHT_ID, id, COLOR_ROLES[choice as usize])
                    .await;

                return Ok(());

                let trigger_status = {
                    let mut members = members.lock().unwrap();
                    members.recent.insert(
                        id,
                        Recent {
                            discrim: event.0.user.discriminator.clone(),
                            id,
                            name: event.0.user.name.clone(),
                        },
                    );

                    match members.recent.len() {
                        0..=2 => TriggerStatus::Inactive,
                        3 => TriggerStatus::ShouldTrigger,
                        _ => TriggerStatus::Triggered,
                    }
                };

                match trigger_status {
                    TriggerStatus::Triggered => {
                        tokio::spawn(update_trigger(members.clone(), state.clone()));
                    }
                    TriggerStatus::ShouldTrigger => {
                        tokio::spawn(trigger(members.clone(), state.clone()));
                    }
                    TriggerStatus::Inactive => {
                        tokio::spawn(timeout(id, members.clone()));
                    }
                }

                tokio::spawn(wait_for_approval(event.0.user, state.clone()));
            }
            _ => {}
        }
    }

    Ok(())
}

async fn wait_for_approval(user: User, state: Arc<State>) {
    let timestamp = user.id.timestamp();
    let mut content = format!(
        "**New user**: <@{}>\nName: {}#{}\nID: {}\nCreation date: {}",
        user.id,
        user.name,
        user.discriminator,
        user.id,
        Utc.timestamp_millis(timestamp).to_rfc3339(),
    );

    let msg_id = state
        .http
        .create_message(JOIN_ID)
        .content(content.clone())
        .unwrap()
        .await
        .unwrap()
        .id;
    let _ = state
        .http
        .create_reaction(
            JOIN_ID,
            msg_id,
            RequestReactionType::Custom {
                id: EMOJI_FERRIS_UP,
                name: Some("ferristhumbsup".to_owned()),
            },
        )
        .await;
    let _ = state
        .http
        .create_reaction(
            JOIN_ID,
            msg_id,
            RequestReactionType::Custom {
                id: EMOJI_FERRIS_DOWN,
                name: Some("ferristhumbsdown".to_owned()),
            },
        )
        .await;
    time::delay_for(Duration::from_secs(3)).await;
    let reaction = state.standby.wait_for_reaction(msg_id, |reaction: &ReactionAdd| {
        matches!(reaction.emoji, ReactionType::Custom { id, .. } if id == EMOJI_FERRIS_DOWN || id == EMOJI_FERRIS_UP)
    }).await.unwrap().0;

    let approved = if matches!(reaction.emoji, ReactionType::Custom { id, ..} if id == EMOJI_FERRIS_DOWN)
    {
        let _ = state.http.remove_guild_member(TWILIGHT_ID, user.id).await;

        false
    } else {
        let _ = state
            .http
            .add_guild_member_role(TWILIGHT_ID, user.id, TWILIGHT_ROLE)
            .await;
        let choice = user.id.0 % 5;
        let _ = state
            .http
            .add_guild_member_role(TWILIGHT_ID, user.id, COLOR_ROLES[choice as usize])
            .await;
        let _ = state
            .http
            .create_message(GENERAL_ID)
            .content(format!("Welcome to Twilight <@{}>", user.id))
            .unwrap()
            .await;

        true
    };

    let _ = write!(
        content,
        "\n\nUser {} by <@{}>",
        if approved { "approved" } else { "denied" },
        reaction.user_id
    );
    let _ = state
        .http
        .update_message(JOIN_ID, msg_id)
        .content(content)
        .unwrap()
        .await;
    let _ = state.http.delete_all_reactions(JOIN_ID, msg_id).await;
}

async fn timeout(id: UserId, members: Arc<Mutex<MemberState>>) {
    time::delay_for(Duration::from_secs(10)).await;

    let mut members = members.lock().unwrap();

    if members.triggered.is_none() {
        members.recent.remove(&id);
    }
}

async fn trigger(members: Arc<Mutex<MemberState>>, state: Arc<State>) {
    let msg = make_message(members.lock().unwrap().recent.values());

    let msg_id = state
        .http
        .create_message(CHANNEL_ID)
        .content(msg)
        .unwrap()
        .await
        .unwrap()
        .id;

    members.lock().unwrap().triggered.replace(msg_id);

    state
        .http
        .create_reaction(
            CHANNEL_ID,
            msg_id,
            RequestReactionType::Unicode {
                name: "👢".to_owned(),
            },
        )
        .await
        .unwrap();
    state
        .http
        .create_reaction(
            CHANNEL_ID,
            msg_id,
            RequestReactionType::Unicode {
                name: "🙂".to_owned(),
            },
        )
        .await
        .unwrap();

    let fut = state.standby.wait_for_reaction(msg_id, |r: &ReactionAdd| {
        matches!(
            &r.0.emoji,
            ReactionType::Unicode { name } if name == "🙂" || name == "👢"
        )
    });
    let reaction = time::timeout(Duration::from_secs(300), fut).await;

    if let Ok(Ok(reaction)) = reaction {
        if matches!(reaction.0.emoji, ReactionType::Unicode { name } if name == "🙂") {
            members.lock().unwrap().recent.clear();
            members.lock().unwrap().triggered.take();

            state
                .http
                .create_message(CHANNEL_ID)
                .content("Users cleared from raid list.")
                .unwrap()
                .await
                .unwrap();

            return;
        }
    }

    let recent_len = members.lock().unwrap().recent.len();

    let banning = state
        .http
        .create_message(CHANNEL_ID)
        .content(format!("Banning {} members... ⏳", recent_len))
        .unwrap()
        .await
        .unwrap()
        .id;

    let mut total = 0usize;

    loop {
        let recent = members.lock().unwrap().recent.clone();

        if recent.is_empty() {
            break;
        }

        for member in recent.values() {
            state
                .http
                .create_ban(TWILIGHT_ID, member.id)
                .delete_message_days(1)
                .unwrap()
                .await
                .unwrap();

            total += 1;
        }
    }

    state
        .http
        .update_message(CHANNEL_ID, banning)
        .content(format!("Finished banning {} members.", total))
        .unwrap()
        .await
        .unwrap();
}

async fn update_trigger(members: Arc<Mutex<MemberState>>, state: Arc<State>) {
    let msg_id = if let Some(id) = members.lock().unwrap().triggered {
        id
    } else {
        return;
    };

    let new_content = make_message(members.lock().unwrap().recent.values());

    state
        .http
        .update_message(CHANNEL_ID, msg_id)
        .content(new_content)
        .unwrap()
        .await
        .unwrap();
}

fn make_message<'a>(members: impl Iterator<Item = &'a Recent>) -> String {
    let mut msg = "Raid detected!\n\nThe list of members below have joined within \
    10 seconds. Do you want to ban all of them? React 👢 to give them the \
    boot or 🙂 to allow them.\n\nReact within 2 minutes, or else they will be banned.\n\n"
        .to_owned();

    for member in members {
        writeln!(
            msg,
            "- <@{}> (name: {}#{}, ID: {})",
            member.id, member.name, member.discrim, member.id
        )
        .unwrap();
    }

    msg
}
