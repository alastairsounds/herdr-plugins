# yazi-popup demo: press a keybinding, pick a file in the popup, watch
# "@path" get typed into a live Claude Code composer -- the plugin's whole
# point is referencing files to a coding agent without leaving the keyboard.
# Keybinding here is ctrl+p, not the README's "prefix+cmd+p" -- see
# _demo/.config/herdr/config.toml for why.
# Playground: DEMO_SAMPLE_REPO, rebuilt fresh from _demo/fixtures/sample-repo
# by `just vhs`'s demo-repo dependency.
# Isolation comes from `just vhs yazi-popup picker.tape` -- don't run
# `vhs picker.tape` directly. Claude Code itself is NOT isolated by that
# (it reads your real, unaffected ~/.claude) -- the tape never sends
# Enter to the composer, so nothing here submits a message or spends a turn.

Require herdr
Require yazi
Require claude

Output picker.gif

Set Shell "zsh"
Set FontFamily "FiraCode Nerd Font"
Set FontSize 20
Set Width 1200
Set Height 600
Set Theme "Catppuccin Mocha"
Set TypingSpeed 30ms
Set Padding 20

Hide
Type "cd $DEMO_SAMPLE_REPO && clear && herdr --session yazi-popup-demo"
Enter
Sleep 1s
Type "claude"
Enter
Sleep 2s

# First run in a new directory: dismiss Claude Code's folder-trust prompt.
# "Yes, I trust this folder" is the default selection.
Enter
Sleep 1s
Show

# A half-typed prompt, like referencing a file mid-message to an agent.
Type "please review "
Sleep 500ms

# Open the picker popup and wait for its file listing to render.
Ctrl+P
Wait+Screen@5s /Cargo.toml/

# src/ is first (dirs-first sort) and already the cursor's position. "l"
# is yazi's "enter" action (navigate in); <Enter> is "open", which in
# chooser mode fires immediately -- on a directory that would select the
# directory itself and close the popup, not navigate into it.
Sleep 500ms
Type "l"
Wait+Screen@5s /main.rs/

# lib.rs is first alphabetically; one down lands on main.rs.
Sleep 500ms
Type "j"
Sleep 400ms
Enter

# Popup closes, "@...main.rs" is typed back into the pane.
Wait+Screen@5s /main.rs/
Sleep 3s
