use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context as _, Result};

use serenity::builder::CreateMessage;
use serenity::client::Context;
use serenity::gateway::ActivityData;
use serenity::model::prelude::{ActivityType, ChannelId, OnlineStatus, Ready, VoiceState};

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

    let record_channel_id = ChannelId::new(
        secrets
            .get("RECORD_CHANNEL_ID")
            .ok_or(anyhow!("`RECORD_CHANNEL_ID` is not provided!"))?
            .parse()
            .context("`RECORD_CHANNEL_ID` is not able to parse!")?,
    );

    let handler = Handler {
        notify_channel_id,
        record_channel_id,
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
    record_channel_id: ChannelId,
    notify_channel_id: ChannelId,
}

#[serenity::async_trait]
impl serenity::client::EventHandler for Handler {
    #[instrument(skip_all, name = "Handler::ready")]
    async fn ready(&self, ctx: Context, ready: Ready) {
        info! {
            ?ready.version,
            ?ready.application.id,
            ?ready.application.flags,
            ready.user.tag = ?ready.user.tag(),
            "handler is ready!",
        };

        match self
            .record_channel_id
            .send_message(
                &ctx,
                CreateMessage::new().content(format!("```\ndeployed: {}\n```", env!("GIT_HASH"))),
            )
            .await
        {
            Ok(_) => (),
            Err(e) => error!(error = ?e, "error occurred while sending message"),
        }

        // FIXME: not reasonal seconds, but required
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;

        let hash_short = env!("GIT_HASH_SHORT");
        let state = if let Some(tagname) = option_env!("GIT_TAG") {
            format!("Running nanai10a/meufchrer @ {tagname} ({hash_short})")
        } else {
            format!("Running nanai10a/meufchrer @ {{no tag}} ({hash_short})")
        };

        let activity = ActivityData {
            // FIXME: this undisplayed but unallown empty string, maybe
            name: "{activity_name}".to_owned(),
            kind: ActivityType::Custom,
            state: Some(state),
            url: None,
        };

        ctx.shard.set_presence(Some(activity), OnlineStatus::Online);
    }

    #[instrument(skip_all, name = "Handler::voice_state_update")]
    async fn voice_state_update(&self, ctx: Context, old: Option<VoiceState>, new: VoiceState) {
        debug!(kind = "voice_state_update", "received gateway event");

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let action = match guess_action(&old, &new) {
            Ok(Some(o)) => o,
            Ok(None) => return, // nothing to do
            Err(e) => return error!(error = ?e, "error occurred while guessing action"),
        };

        tokio::join! {
            self.handle_as_record(&ctx, &old, &new, timestamp, &action),
            self.handle_as_notify(&ctx, &old, &new, timestamp, &action),
        };
    }
}

impl Handler {
    #[instrument(skip_all, name = "Handler::handle_as_record")]
    async fn handle_as_record(
        &self,
        ctx: &Context,
        _: &Option<VoiceState>,
        new: &VoiceState,
        timestamp: u64,
        _: &Action,
    ) {
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
                t: {timestamp}
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
            .record_channel_id
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

    #[instrument(skip_all, name = "Handler::handle_as_notify")]
    async fn handle_as_notify(
        &self,
        ctx: &Context,
        _: &Option<VoiceState>,
        new: &VoiceState,
        timestamp: u64,
        action: &Action,
    ) {
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

        let action = match action {
            Action::Join { into } => format!("join into <#{into}>"),
            Action::Move { from, into } => format!("move from <#{from}> into <#{into}>"),
            Action::Leave { from } => format!("leave from <#{from}>"),
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

fn guess_action(old: &Option<VoiceState>, new: &VoiceState) -> Result<Option<Action>> {
    if let Some(true) = old
        .as_ref()
        .map(|old| old.session_id == new.session_id && old.channel_id == new.channel_id)
    {
        return Ok(None);
    }

    let pattern = (old.as_ref().map(|vs| vs.channel_id), new.channel_id);

    match pattern {
        (Some(Some(from)), Some(into)) => Ok(Some(Action::Move { from, into })),
        (Some(Some(from)), None) => Ok(Some(Action::Leave { from })),
        (None, Some(into)) => Ok(Some(Action::Join { into })),

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
