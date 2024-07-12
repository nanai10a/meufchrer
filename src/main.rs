use anyhow::Context as _;
use anyhow::anyhow;

use serenity::prelude::Context;
use serenity::model::prelude::Ready;

use tracing::{info, instrument};

#[shuttle_runtime::main]
async fn serenity(#[shuttle_runtime::Secrets] secrets: shuttle_runtime::SecretStore) -> shuttle_serenity::ShuttleSerenity {
    let token = secrets.get("DISCORD_TOKEN").ok_or(anyhow!("`DISCORD_TOKEN` is not provided!"))?;
    let intents = serenity::prelude::GatewayIntents::empty();

    let handler = Handler;

    let client = serenity::Client::builder(&token, intents)
        .event_handler(handler)
        .await
        .context("failed to initialize Discord client")?;

    Ok(client.into())
}

struct Handler;

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
}
