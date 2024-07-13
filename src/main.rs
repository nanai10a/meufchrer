use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context as _, Result};

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
            member: Some(member),
            ..
        } = &new
        else {
            return error!("member is not present!");
        };

        let name = if let Some(nick) = &member.nick {
            &format!("{nick} ({username})", username = &member.user.name)
        } else {
            &member.user.name
        };

        let action = match guess_action(old, new) {
            Ok(Action::Join { into }) => {
                format!("join into <#{into}>")
            }
            Ok(Action::Move { from, into }) => {
                format!("move from <#{from}> into <#{into}>")
            }
            Ok(Action::Leave { from }) => {
                format!("leave from <#{from}>")
            }
            Err(e) => {
                return error!(error = ?e, "error occurred while guessing action");
            }
        };

        let content = format!("{name} {action} at <t:{timestamp}:R>");

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

fn guess_action(old: &Option<VoiceState>, new: &VoiceState) -> Result<Action> {
    let pattern = (old.as_ref().map(|vs| vs.channel_id), new.channel_id);

    match pattern {
        (Some(Some(from)), Some(into)) => Ok(Action::Move { from, into }),
        (Some(Some(from)), None) => Ok(Action::Leave { from }),
        (None, Some(into)) => Ok(Action::Join { into }),

        (Some(None), Some(_)) => Err(anyhow!("unexpected pattern: {pattern:?}")),
        (Some(None), None) => Err(anyhow!("unexpected pattern: {pattern:?}")),
        (None, None) => Err(anyhow!("unexpected pattern: {pattern:?}")),
    }
}

enum Action {
    Join { into: ChannelId },
    Leave { from: ChannelId },
    Move { from: ChannelId, into: ChannelId },
}
