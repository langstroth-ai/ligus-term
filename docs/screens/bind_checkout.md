# Binding this device's checkout for apply on host

Rendered by `tests/screens.rs` (110×32, the in-memory demo backend).

```text
[NA] [DS]• Northwind API · owner │ Chats Workflows Props Agents Hosts Memory People Notifs 1 Settings
┏ Workers & hosts ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃  Host                 Kind    Status     Work  Agents                                          Repos       ┃
┃● This device          DEVICE  online     idle  @primary @reviewer                                          ┃
┃● sff                  WORKER  online     busy  @coder @scout                                   northwind-ap┃
┃○ ada-studio           DEVICE  3h ago     idle                                                              ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                  ┏ This device's checkout ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓                  ┃
┃                  ┃The git checkout approved proposals are applied in when a host asks   ┃                  ┃
┃                  ┃this device.                                                          ┃                  ┃
┃                  ┃/srv/northwind-api                                                    ┃                  ┃
┃                  ┃                                                                      ┃                  ┃
┃                  ┃enter save · empty to unbind · esc cancel                             ┃                  ┃
┃                  ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛                  ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┃                                                                                                            ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
No checkout bound: this device can't apply proposals here. b bind one
A host runs agents and applies approved proposals on its checkout. A worker is headless.

↑↓ hosts  b bind checkout  f1 help                                   peer to peer · lt on this terminal · demo
```
