# 太阳系内核

**目标：** 让 STKCraft 的数值内核不只懂地球——时间尺度覆盖 TDB，天体参数与自转模型按 IAU 决议，
行星/月球位置有可验证的来源，场景能指定中心天体，于是"切换到任何一颗行星"是内核的能力而不是
界面上的假象。

**主张（与 `AGENTS.md` 同一标准）：** 每个数值层都要对上公开参考。参考优先级：原始决议与标准
（IAU、IERS、NAIF、USNO）> 标准参考实现（IAU SOFA / ERFA、JPL Horizons、NAIF 内核）> 论文。
"看起来对"不是证据。

## 实现来源与许可

- **自己实现**：不依赖 `nyx-space/anise`、`nyx-space/hifitime`（两者 MPL-2.0）或 `nyx`
  （AGPL-3.0），也不拷贝其源码。它们只作为设计参考阅读。
- **数据来源**：NAIF 的公开文本内核（`pck00011.tpc`、`gm_de440.tpc`）与 IAU/WGCCRE 报告。
  数值以生成式表格进源码，每张表带 `出处 + 版本` 注释。
- **验证来源**：ERFA（BSD，IAU SOFA 派生）与 JPL Horizons。测试向量带查询条件与日期。

## 五个阶段

每段独立通过 `cargo xtask ci` 并单独提交。

### 段 1 — 时间层补全 TDB（`stkcraft-time`）

- `TDB − TT` 用 Fairhead & Bretagnon (1990) 的前七项截断级数，系数见 Kaplan (2005) USNO
  Circular 179；文档给的范围是 1600–2200 年优于 10 µs。
- 方向：`TT → TDB` 直接加；`TDB → TT` 因修正量依赖于 TT，用一次不动点迭代（收敛到亚微秒）。
- 与现有 `to_scale` 的"经 TAI 中转"结构一致：`Tdb` 不再等同 `Tt`。
- **验收**：8 个纪元（1985…2025）对 ERFA `eraDtdb` 的差 < 20 µs；振幅界 < 1.7 ms；往返误差 < 1 µs。

### 段 2 — 新建 `stkcraft-body`（L1，位于 time 与 frames 之间）

- `Body`：Sun、Mercury、Venus、Earth、Moon、Mars、Jupiter、Saturn、Uranus、Neptune；NAIF ID 与
  IAU 名双键（为段 3 的内核后端留接口）。
- **GM**：DE440 数值（`gm_de440.tpc`，Park et al. 2021, AJ 161:105），单位 km³/s²。
- **半径与椭球**：IAU WGCCRE 2015（Archinal et al. 2018, CMDA 130:22），来源
  `pck00011.tpc` 的 `BODYnnn_RADII`。
- **自转模型**：IAU 2015 —— 极轴 α₀、δ₀ 与主经线 W 的多项式（每世纪多项式，J2000 起算），
  月球含天平动周期项；由此得到 `body_fixed ↔ ICRF` 的 3-1-3 旋转。
- **行星心 / 行星大地**：每体用自己的扁率椭球，不用 WGS-84 硬编码。
- **验收**：Archinal et al. 2018 表格在 J2000 的 W 与极点值；旋转矩阵正交性与往返；椭球上的
  点位往返误差 < 1 m。

### 段 3 — 新建 `stkcraft-ephemeris`（L2）

- `trait Ephemeris { fn state(&self, body: Body, t: Epoch) -> Result<StateVector>; }`
  返回 ICRF 位置/速度，时间参数 TDB。
- **后端 A（内置，离线）**：VSOP87 截断（Bretagnon & Francou 1988）给行星日心黄道位置；
  ELP2000-82B 截断给月球；用 IAU 2006 黄赤交角转到 ICRF。表是生成式小表，不含数据文件。
- **后端 B（可选）**：自实现 NAIF DAF/SPK 读取器（Type 2/3），吃 `de440s.bsp`（32 MB）与
  JPL 卫星内核。数据不进仓库，按固定 SHA-256 拉取；离线 CI 跳过且不失败。
- **验收**：两后端对 JPL Horizons 的公开查询值；A 与 B 的互差要在 A 的标称精度内。

### 段 4 — 引擎的中心天体（`stkcraft-engine`）

- `Scenario` 增加 `central_body`；卫星除 TLE（仅地球）外支持行星中心二体 + J2/J4；站点按该天体
  椭球与固连系；遮蔽用该天体半径；命令与场景文档同步。
- **验收**：手算 Kepler 解；固连系往返；地球路径结果逐位不变（回归）。

### 段 5 — 行星系统（卫星）

- 火卫、伽利略四星、Titan 等，中心天体为其母行星，精度取自内核；解析后端对其返回
  `Error::Unsupported`，不猜。

## 不做的事

- 不做行星际转移轨道设计与行星摄动；行星范围内二体 + J2/J4，不假装能做混沌积分。
- 不把 ET/UT1 之外的地球定向参数补齐到 IERS 全量（那是独立的一段）。

## 每段的错误处理

沿用 `AGENTS.md`：无 panic，未实现的能力返回 `Error::Unsupported`，输入先校验，NaN 用 `Option`
表达。GPU 与天文数值同样对待。
