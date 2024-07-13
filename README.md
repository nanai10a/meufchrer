# meufchrer

*what?: [meufchrer - This Word Does Not Exist](https://l.thisworddoesnotexist.com/MVNf)*

built for [Approvers](https://github.com/approvers)

---

## how to use

this project is for [Shuttle](https://www.shuttle.rs/), so [`cargo-shuttle`](https://crates.io/crates/cargo-shuttle) is required.

list secrets on below:

| key                 | description                 | required |
| ------------------- | --------------------------- | -------- |
| `DISCORD_TOKEN`     | token of Discord Bot        | yes      |
| `NOTIFY_CHANNEL_ID` | channel id for notification | yes      |
| `RECORD_CHANNEL_ID` | channel id for recording    | yes      |

## what's this

this is a alternative of vcdiff that is part of [`approvers/rusty-ponyo`](https://github.com/approvers/rusty-ponyo).

## notice

further reading: [Idle Projects - Shuttle](https://docs.shuttle.rs/getting-started/idle-projects)

deployer may should do this:

```sh
cargo shuttle restart meufchrer --idle-minutes 0
```

because Shuttle stops low-load projects automatically by default, so needs specify `--idle-minutes` manually if this project runs permanently.

*more details are write later...*
