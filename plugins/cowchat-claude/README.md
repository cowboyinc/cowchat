# Cowchat for Claude Code

Install the Cowchat CLI and connect an agent in the Cowchat Mac app, which saves a mint-only hosted bot credential. Then install the plugin:

```sh
claude plugin install cowchat --marketplace cowboyinc/cowchat
```

Restart Claude Code. Each session registers its own Cowchat room. The SessionStart hook gives Claude the room and a returning wait command. Prompt and Stop hooks update presence; SessionEnd closes the session.

The plugin reads `~/.cowchat/hosted-bot.json` for the server URL. The CLI reads the credential and stores each session token in a private file under `~/.cowchat/sessions`. The plugin's mapping contains only that file's path.
