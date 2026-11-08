## Active
- [ ] Clipboard: load, copy, paste #clipboard <- [editor-wrap, tui-dummy]
- [ ] Rewrite validation and state machine #rewrite-flow <- [clipboard, marker-extract]
- [ ] Prompt template and instruction delivery #prompt-template <- [rewrite-flow]
- [ ] Session id lifecycle and Ctrl+N #session-lifecycle <- [rewrite-flow]
- [ ] pi adapter #pi-adapter <- [harness-trait, session-lifecycle]
- [ ] Robustness: cancel, timeout, resize, error paths #robustness <- [session-lifecycle]
- [ ] Remove the dead_code allowance from src/editor.rs #editor-dead-code <- [editor-wiring]

## Closed
- [x] Bootstrap harness: AGENTS.md + Makefile #harness-bootstrap <- [tui-dummy]
- [x] Dummy empty TUI app #tui-dummy
- [x] CLI args and startup validation #args-cli <- [tui-dummy]
- [x] Editor core: buffer, cursor, edit operations #editor-widget <- [layout-panes, tui-dummy]
- [x] Four-pane layout and focus cycling #layout-panes <- [args-cli, tui-dummy]
- [x] Status line and quit paths #status-quit <- [tui-dummy]
- [x] Harness trait and claude argv builder #harness-trait <- [harness-bootstrap]
- [x] Marker extraction and error states #marker-extract <- [pane-routing]
- [x] Output pane from final text, debug pane from stream #pane-routing <- [editor-wrap, process-spawn]
- [x] Process spawn and stream readers #process-spawn <- [harness-trait]
- [x] Editor wrapping: soft-wrap rendering and wrap-aware movement #editor-wrap <- [editor-widget]
- [x] Apache 2.0 licence: Yury Antonov, 2026 #license-apache <- [tui-dummy]
- [x] Makefile install target: release build, stamp git hash, copy to ~/.local/bin #zzfl3
- [x] Wire Editor and wrap into the editable panes #editor-wiring <- [editor-wrap, pane-routing]
- [x] Pin LF line endings with .gitattributes #gitattributes <- [tui-dummy]
