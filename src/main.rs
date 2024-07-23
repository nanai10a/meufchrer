use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context as _, Result};

use serenity::builder::CreateMessage;
use serenity::client::{Client, Context};
use serenity::gateway::ActivityData;
use serenity::model::prelude::{ActivityType, ChannelId, OnlineStatus, Ready, VoiceState};

use tracing::{debug, error, info, instrument};

use shuttle_runtime::{
    CustomError as ShuttleCustomError, Error as ShuttleError, Service as ShuttleService,
};

type ShuttleResult<T> = Result<T, ShuttleError>;

use futures::TryFutureExt as _;

use axum::{routing, Router};

use tokio::net::TcpListener;

#[shuttle_runtime::main]
async fn serenity(
    #[shuttle_runtime::Secrets] secrets: shuttle_runtime::SecretStore,
) -> ShuttleResult<impl ShuttleService> {
    STARTUP_TIME.get_or_init(SystemTime::now);

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

    let router = Router::new()
        .route("/", routing::get(console::index))
        .route("/socket", routing::get(console::socket))
        .route(
            "/_htmx/deployments",
            routing::get(console::htmx::deployments),
        );

    Ok(Service { client, router })
}

static STARTUP_TIME: OnceLock<SystemTime> = OnceLock::new();

mod console {
    use axum::extract::WebSocketUpgrade;
    use axum::response::IntoResponse;
    use axum::response::Response;

    pub async fn index() -> impl IntoResponse {
        Response::builder()
            .header("content-type", "text/html")
            .body(include_str!("../assets/index.html").to_owned())
            .unwrap()
    }

    pub async fn socket(wsu: WebSocketUpgrade) -> impl IntoResponse {
        wsu.on_upgrade(|ws| async {
            // TODO: ...needs this?
        })
    }

    pub mod htmx {
        use super::*;

        use std::time::SystemTime;

        pub async fn deployments() -> impl IntoResponse {
            let uptime = crate::duration_display(
                SystemTime::now()
                    .duration_since(*crate::STARTUP_TIME.get().unwrap())
                    .unwrap(),
            );

            Response::builder()
                .header("content-type", "text/html")
                .body(format!(
                    "<p>tag: {tag}</p><p>hash: <a href=\"https://github.com/nanai10a/meufchrer/tree/{hash}\">{hash}</a></p><p>uptime: {uptime}</p>",
                    tag = option_env!("GIT_TAG").unwrap_or("{no tag}"),
                    hash = env!("GIT_HASH"),
                ))
                .unwrap()
        }
    }
}

struct Service {
    client: Client,
    router: Router,
}

#[serenity::async_trait]
impl ShuttleService for Service {
    async fn bind(self, addr: SocketAddr) -> ShuttleResult<()> {
        let Self { mut client, router } = self;

        let serenity = tokio::spawn(async move {
            client
                .start_autosharded()
                .map_err(ShuttleCustomError::new)
                .await
        })
        .map_err(ShuttleCustomError::new);

        let axum = tokio::spawn(async move {
            axum::serve(TcpListener::bind(addr).await?, router)
                .into_future()
                .map_err(ShuttleCustomError::new)
                .await
        })
        .map_err(ShuttleCustomError::new);

        tokio::select! {
            Ok(Err(e)) | Err(e) = axum => Err(e)?,
            Ok(Err(e)) | Err(e) = serenity => Err(e)?,
        }
    }
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
            Some(o) => o,
            None => return, // nothing to do
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
            Action::Join { into } => format!("joined into <#{into}>"),
            Action::Move { from, into } => format!("moved from <#{from}> into <#{into}>"),
            Action::Leave { from } => format!("leaved from <#{from}>"),
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

fn guess_action(old: &Option<VoiceState>, new: &VoiceState) -> Option<Action> {
    let is_same_session = old.as_ref().map(|old| &old.session_id) == Some(&new.session_id);

    let old_channel_id = old.as_ref().map(|old| old.channel_id).flatten();
    let new_channel_id = new.channel_id;

    match (old_channel_id, new_channel_id) {
        (None, Some(into)) if !is_same_session => Some(Action::Join { into }),
        (Some(from), None) if is_same_session => Some(Action::Leave { from }),
        (Some(from), Some(into)) if from == into => Some(Action::Move { from, into }),
        _ => None,
    }
}

enum Action {
    Join { into: ChannelId },
    Leave { from: ChannelId },
    Move { from: ChannelId, into: ChannelId },
}

fn duration_display(duration: Duration) -> String {
    let secs = duration.as_secs();
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    let weeks = days / 7;
    let months = weeks / 4;
    let years = months / 12;

    [
        (years, "years", 0),
        (months, "months", 12),
        (weeks, "weeks", 4),
        (days, "days", 4),
        (hours, "hours", 24),
        (mins, "mins", 60),
        (secs, "secs", 60),
    ]
    .windows(2)
    .find_map(|window| {
        let [(n0, u0, _), (n1, u1, q1)] = window else {
            unreachable!();
        };

        if *n0 == 0 {
            return None;
        }

        Some(format!("{n0} {u0}, {n1} {u1}", n1 = n1 % q1))
    })
    .unwrap_or(format!("{:.3} secs", duration.as_secs_f64()))
}
