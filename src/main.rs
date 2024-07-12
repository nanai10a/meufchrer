use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use anyhow::Context as _;

use serenity::builder::CreateMessage;
use serenity::model::prelude::{ChannelId, Ready, VoiceState};
use serenity::prelude::Context;

use tracing::{debug, error, info, instrument};

#[shuttle_runtime::main]
async fn serenity(
    #[shuttle_runtime::Secrets] secrets: shuttle_runtime::SecretStore,
) -> shuttle_serenity::ShuttleSerenity {
    let token = secrets
        .get("DISCORD_TOKEN")
        .ok_or(anyhow!("`DISCORD_TOKEN` is not provided!"))?;

    let notify_channel_id = ChannelId::new(
        secrets
            .get("NOTIFY_CHANNEL_ID")
            .ok_or(anyhow!("`NOTIFY_CHANNEL_ID` is not provided!"))?
            .parse()
            .context("`NOTIFY_CHANNEL_ID` is not able to parse!")?,
    );

    let logging_channel_id = ChannelId::new(
        secrets
            .get("LOGGING_CHANNEL_ID")
            .ok_or(anyhow!("`LOGGING_CHANNEL_ID` is not provided!"))?
            .parse()
            .context("`LOGGING_CHANNEL_ID` is not able to parse!")?,
    );

    let handler = Handler {
        notify_channel_id,
        logging_channel_id,
    };

    let intents = serenity::prelude::GatewayIntents::GUILDS
        | serenity::prelude::GatewayIntents::GUILD_VOICE_STATES;

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

        tokio::join! {
            self.handle_as_record(&ctx, &old, &new),
            self.handle_as_notification(&ctx, &old, &new),
        };
    }
}

impl Handler {
    #[instrument(skip_all, name = "Handler::handle_as_record")]
    async fn handle_as_record(&self, ctx: &Context, _: &Option<VoiceState>, new: &VoiceState) {
        let VoiceState {
            guild_id: Some(guild_id),
            channel_id,
            user_id,
            session_id,
            ..
        } = &new
        else {
            return error!("guild_id is not present!");
        };

        let channel_id = channel_id
            .map(|v| v.to_string())
            .unwrap_or("{empty}".to_owned());

        let content = format!(
            "
                ```
                v: 0
                g: {guild_id}
                u: {user_id}
                s: {session_id}
                c: {channel_id}
                ```
            ",
        )
        .replace("\n                ", "\n")
        .trim()
        .to_owned();

        match self
            .logging_channel_id
            .send_message(&ctx, CreateMessage::new().content(content))
            .await
        {
            Ok(msg) => {
                debug!(message_id = ?msg.id, "successfully sent");
            }
            Err(e) => {
                error!(error = ?e, "error occurred while sending message");
            }
        }
    }

    #[instrument(skip_all, name = "Handler::handle_as_notification")]
    async fn handle_as_notification(
        &self,
        ctx: &Context,
        old: &Option<VoiceState>,
        new: &VoiceState,
    ) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let VoiceState {
            channel_id,
            member: Some(member),
            ..
        } = &new
        else {
            return error!("member is not present!");
        };

        let Some(ensured_channel_id) =
            channel_id.or_else(|| old.clone().map(|vs| vs.channel_id).flatten())
        else {
            return error!("cannot ensure channel_id!");
        };

        let name = if let Some(nick) = &member.nick {
            &format!("{nick} ({username})", username = &member.user.name)
        } else {
            &member.user.name
        };

        let (verb, prep) = if channel_id.is_some() {
            ("join", "into")
        } else {
            ("leave", "from")
        };

        let content = format!("{name} {verb} {prep} <#{ensured_channel_id}> at <t:{timestamp}:R>");

        match self
            .notify_channel_id
            .send_message(&ctx, CreateMessage::new().content(content))
            .await
        {
            Ok(msg) => {
                debug!(message_id = ?msg.id, "successfully sent");
            }
            Err(e) => {
                error!(error = ?e, "error occurred while sending message");
            }
        }
    }
}
