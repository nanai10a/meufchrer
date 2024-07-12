use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use anyhow::anyhow;

use serenity::prelude::Context;
use serenity::model::prelude::{Ready, ChannelId, VoiceState};
use serenity::builder::CreateMessage;

use tracing::{debug, error, info, instrument};

#[shuttle_runtime::main]
async fn serenity(#[shuttle_runtime::Secrets] secrets: shuttle_runtime::SecretStore) -> shuttle_serenity::ShuttleSerenity {
    let token = secrets.get("DISCORD_TOKEN").ok_or(anyhow!("`DISCORD_TOKEN` is not provided!"))?;

    let notify_channel_id = secrets.get("NOTIFY_CHANNEL_ID").ok_or(anyhow!("`NOTIFY_CHANNEL_ID` is not provided!"))?;
    let notify_channel_id = ChannelId::new(notify_channel_id.parse().context("`NOTIFY_CHANNEL_ID` is not able to parse!")?);

    let logging_channel_id = secrets.get("LOGGING_CHANNEL_ID").ok_or(anyhow!("`LOGGING_CHANNEL_ID` is not provided!"))?;
    let logging_channel_id = ChannelId::new(logging_channel_id.parse().context("`LOGGING_CHANNEL_ID` is not able to parse!")?);

    let handler = Handler { notify_channel_id, logging_channel_id };

    let intents = serenity::prelude::GatewayIntents::GUILDS | serenity::prelude::GatewayIntents::GUILD_VOICE_STATES;

    let client = serenity::Client::builder(&token, intents)
        .event_handler(handler)
        .await
        .context("failed to initialize Discord client")?;

    Ok(client.into())
}

struct Handler {
    logging_channel_id: ChannelId,
    notify_channel_id: ChannelId,
}

#[serenity::async_trait]
impl serenity::client::EventHandler for Handler {
    #[instrument(skip_all, name = "Handler::ready")]
    async fn ready(&self, _: Context, ready: Ready) {
        info! {
            ?ready.version,
            ?ready.application.id,
            ?ready.application.flags,
            ready.user.tag = ?ready.user.tag(),
            "handler is ready!",
        };
    }

    #[instrument(skip_all, name = "Handler::voice_state_update")]
    async fn voice_state_update(&self, ctx: Context, old: Option<VoiceState>, new: VoiceState) {
        debug!(kind = "voice_state_update", "received gateway event");

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        let VoiceState { guild_id, channel_id, user_id, session_id, member, .. } = new;

        let action_verb = if channel_id.is_some() { "join" } else { "leave" };
        let action_prep = if channel_id.is_some() { "into" } else { "from" };

        let ensured_channel_id = channel_id.unwrap_or_else(|| old.unwrap().channel_id.unwrap());

        let guild_id = guild_id.map(|v| v.to_string()).unwrap_or("{empty}".to_owned());
        let channel_id = channel_id.map(|v| v.to_string()).unwrap_or("{empty}".to_owned());
        let member = member.unwrap();

        let username = member.user.name;
        let name = if let Some(nick) = member.nick {
            format!("{nick} ({username})")
        } else {
            username
        };

        {
            let content = format!("{name} {action_verb} {action_prep} <#{ensured_channel_id}> at <t:{timestamp}:R>");

            let result = self.logging_channel_id.send_message(&ctx, CreateMessage::new().content(content)).await;
            match result {
                Ok(msg) => {
                    debug!(kind = "log", message_id = ?msg.id, "successfully sent");
                },
                Err(e) => {
                    error!(kind = "log", error = ?e, "error occurred while sending message");
                },
            }
        }

        {
            let content = format!("
                ```
                v: 0
                g: {guild_id}
                u: {user_id}
                s: {session_id}
                c: {channel_id}
                ```
            ").replace("\n                ", "").trim().to_owned();

            let result = self.notify_channel_id.send_message(&ctx, CreateMessage::new().content(content)).await;
            match result {
                Ok(msg) => {
                    debug!(kind = "notify", message_id = ?msg.id, "successfully sent");
                },
                Err(e) => {
                    error!(kind = "notify", error = ?e, "error occurred while sending message");
                },
            }
        }
    }
}
