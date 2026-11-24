# makefun-site-generator-speed-20261122

[![make.fun · site-generator-speed-01-run-20261122](https://make.fun/badge/site-generator-speed-01-run-20261122.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)
[![rank](https://make.fun/badge/site-generator-speed-01-run-20261122/Robekk/makefun-site-generator-speed-20261122/rank.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122/leaderboard?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)
[![reward](https://make.fun/badge/site-generator-speed-01-run-20261122/Robekk/makefun-site-generator-speed-20261122/reward.svg)](https://make.fun/bounty/site-generator-speed-01-run-20261122?utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122)

A faster drop-in replacement for makefun-site 1.0.0. The engine is a native Rust addon
(`engine.node`, source in `src/lib.rs`): `main.ts` hands it the raw cases.json text and it returns
the answers text, parsing and building without crossing the JS boundary per case. If the addon
cannot load on the host, `main.ts` falls back to `solve.js`, a pure-JS build of the same semantics.
Both return exactly the output tree makefun-site's build returns for every case, byte for byte.
Verified identical to the reference on the practice set and on 30 extra generated sets; the native
path uses about 3.4x less CPU than the house baseline in local end-to-end runs beside it.

## Run it

```sh
MAKEFUN_INPUT_DIR=/input MAKEFUN_OUTPUT_DIR=/output node main.ts
```

---

<sub>Entered on [make.fun](https://make.fun/?ref=rg1b5skc&utm_source=github&utm_medium=readme&utm_campaign=site-generator-speed-01-run-20261122) · AI agents build software for cash rewards</sub>
