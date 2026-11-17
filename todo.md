# TODO

## VST2 → VST3 官方支持：要提供的插件

每个插件要一个 FL 工程：同一个插件的 **VST2 和 VST3 各放一个**，载入**同一个预设**（最好再改几个参数，
两边改成一样），保存。我对比两个实例的存档（record 52/53），写出转换规则和模板，再在 FL 里验证。

| 插件 | 厂商 | VST3 ID（本机 FL 插件库） | 工程 | 状态 |
|---|---|---|---|---|
| BBC Symphony Orchestra | Spitfire Audio | `535453566e61627462632073796d7068`（Steinberg 规则） | | 待提供 |
| FabFilter Pro-Q 3 | FabFilter | `71dbc4724d7a9a45b97e51745d84b39d`（不按 Steinberg 规则） | | 待提供 |
| FabFilter Pro-R | FabFilter | `30b56e03592ae211a28170076288709b`（不按 Steinberg 规则） | | 待提供 |
| ValhallaRoom | Valhalla DSP | `525453567575766d616c68616c6c6172`（Steinberg 规则） | | 待提供 |
| （截图里的第 5 个） | | | | 待提供 |

已完成：Serum、OTT（含旧版 OTT VST3 的 ID 修复）。

Live 工程（.als）里的 VST2 → VST3：`supply 1.als` 加了 19 个（FabFilter 新一代 6 个、Valhalla 9 个、
soothe2、PITCHMAP、Vital、Blackhole），已在 Live 验证（重新保存后和插件自己的 VST3 逐字节一致）。
`supply 2.als` 加了 FabFilter 老一代：11 个已在 Live 验证（重新保存后和同预设的 VST3 一致）；
Volcano 2、Pro-G 有几个档位差一格（默认不勾）；Simplon 的 VST3 不认交接，已去掉。
来源和细节见 `docs/plugins.md`。上表是 FL 工程要的。

## 其他

- Serum → Serum 2（升级插件）：需要一个工程，两个插件同一个音色。
- Ample Guitar 官方规则：需要新版插件存的 Live 工程，对照旧存档。
- FL 里还没试过：Kontakt VST2 → Kontakt 8、Disperser / Decapitator 的设置。
