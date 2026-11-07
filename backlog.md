## Active
- [ ] Bootstrap harness: AGENTS.md + Makefile #harness-bootstrap <- [tui-dummy]
- [ ] Dummy empty TUI app #tui-dummy
- [ ] CLI args and startup validation #args-cli <- [tui-dummy]
- [ ] Clipboard: load, copy, paste #clipboard <- [editor-wrap, tui-dummy]
- [ ] Editor core: buffer, cursor, edit operations #editor-widget <- [layout-panes, tui-dummy]
- [ ] Four-pane layout and focus cycling #layout-panes <- [args-cli, tui-dummy]
- [ ] Status line and quit paths #status-quit <- [tui-dummy]
- [ ] Harness trait and claude argv builder #harness-trait <- [harness-bootstrap]
- [ ] Marker extraction and error states #marker-extract <- [pane-routing]
- [ ] Output pane from final text, debug pane from stream #pane-routing <- [editor-wrap, process-spawn]
- [ ] Process spawn and stream readers #process-spawn <- [harness-trait]
- [ ] Rewrite validation and state machine #rewrite-flow <- [clipboard, marker-extract]
- [ ] Prompt template and instruction delivery #prompt-template <- [rewrite-flow]
- [ ] Session id lifecycle and Ctrl+N #session-lifecycle <- [rewrite-flow]
- [ ] Editor wrapping: soft-wrap rendering and wrap-aware movement #editor-wrap <- [editor-widget]
- [ ] Apache 2.0 licence: Yury Antonov, 2026 #license-apache <- [tui-dummy]
- [ ] pi adapter #pi-adapter <- [harness-trait, session-lifecycle]
- [ ] Robustness: cancel, timeout, resize, error paths #robustness <- [session-lifecycle]

## Closed
