# herdr-starship demo: sidebar renders git + rust state as soon as a
# workspace is focused -- hooks fire automatically, no keys needed.
# Playground: DEMO_SAMPLE_REPO, a git repo with dirty state rebuilt fresh
# from _demo/fixtures/sample-repo by `just vhs`'s demo-repo dependency.
# Isolation (XDG_CONFIG_HOME/XDG_STATE_HOME, pane-var stripping) comes from
# `just vhs starship sidebar.tape` -- don't run `vhs sidebar.tape` directly.

Require herdr

# Static payoff (nothing animates once the sidebar settles), so we skip the
# gif and grab a screenshot directly.

Set Shell "zsh"
Set FontFamily "FiraCode Nerd Font"
Set FontSize 20
Set Width 1150
Set Height 300
Set Theme "Catppuccin Mocha"
Set TypingSpeed 30ms
Set Padding 20

Hide
Type "cd $DEMO_SAMPLE_REPO && clear && herdr --session herdr-starship-demo"
Enter
Sleep 1s
Show

# Give hooks a moment to push the sidebar token, then hold on the payoff.
Wait+Screen@10s /main/
Sleep 3s
Screenshot sidebar.png
Sleep 500ms
