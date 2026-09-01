# tally demo: sidebar $num tokens dynamically renumber themselves on workspace
# creation and closure, without any manual step.
# Playground: DEMO_SAMPLE_REPO, a git repo rebuilt fresh from
# _demo/fixtures/sample-repo by `just vhs`'s demo-repo dependency.
# Isolation (XDG_CONFIG_HOME/XDG_STATE_HOME, pane-var stripping) comes from
# `just vhs tally numbering.tape` -- don't run `vhs numbering.tape` directly.

Require herdr

Output numbering.gif

Set Shell "zsh"
Set FontFamily "FiraCode Nerd Font"
Set FontSize 20
Set Width 1150
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 30ms
Set Padding 20

Hide
Type "cd $DEMO_SAMPLE_REPO && clear && herdr --session tally-demo"
Enter
Sleep 1s
Show

# Startup hook stamps $num on the one workspace that exists so far.
Wait+Screen@10s /\.sample-repo · 1/
Sleep 250ms

# new_workspace
Ctrl+N
Wait+Screen@5s /\.sample-repo · 2/
Sleep 250ms

Ctrl+N
Wait+Screen@8s /\.sample-repo · 3/
Sleep 500ms

# previous_workspace (from 3 to 2)
Ctrl+K
Sleep 500ms

# close_workspace (workspace 3 renumbered to 2)
Ctrl+D
Sleep 1s
Enter
Sleep 500ms
