# Mac 验证证据（MAC_REPORT.md 的附录）

本文件收录 `docs/mac/MAC_REPORT.md` 中每个结论所依据的原始输出。原始日志在 `target/mac/` 和 `target/mac-run1/`，按规则不提交：完整日志的比较说明里含有 Modeler 的输出值。这里只放逐段摘录，每段都确认过只含退出码、汇总行、路径、节点 id、错误信息，或我们自己引擎算出的值，不含 Modeler 的输出值。路径里的 `/Users/mac/dev/data-mining-software/` 已去掉。

- 第一轮：子代理运行，代码为 8a6a046，日志在 `target/mac-run1/`。
- 第二轮：协调者运行，代码含两个平台修复，日志在 `target/mac/`。

## 目录

1. [环境：unixODBC 链接错误](#1-环境unixodbc-链接错误)
2. [子代理的执行记录和原始汇报](#2-子代理的执行记录和原始汇报)
3. [平台问题：内联模块里的 `#[path]`](#3-平台问题内联模块里的-path)
4. [平台问题：跨标签的数据引用](#4-平台问题跨标签的数据引用)
5. [修复后的回归与基线逐流对照](#5-修复后的回归与基线逐流对照)
6. [time_series 单元测试：分叉点与 bisect](#6-time_series-单元测试分叉点与-bisect)
7. [test-corpus 的其余失败](#7-test-corpus-的其余失败)
8. [复现命令](#8-复现命令)

## 1. 环境：unixODBC 链接错误

在新 Mac 上首次运行 `cargo xtask corpus verify`（构建 xtask）时：

```
  = note: ld: warning: search path '/Users/mac/lib' not found
          ld: library 'odbc' not found
          clang: error: linker command failed with exit code 1 (use -v to see invocation)

error: could not compile `xtask` (bin "xtask") due to 1 previous error
```

链接命令里有 `"-lodbc" "-liconv" … "-L" "/opt/homebrew/lib"`。来源是 `crates/io/Cargo.toml` 的 `odbc-api = { version = "=29.2.0", … }`（`Cargo.lock`：`odbc-sys 0.31.0`）。`brew install unixodbc`（2.3.14）之后重跑：

```
xtask corpus: 1717 files ok (all labels)
```

## 2. 子代理的执行记录和原始汇报

模型 `opencode/glm-5.3-flash-free`，opencode v2.2.6，启动命令：

```sh
opencode run --standalone --auto -m 'opencode/glm-5.3-flash-free' --format json --title mac-agent \
  "Read docs/mac/SUBAGENT_BRIEF.md and carry out exactly what it says."
```

正常退出（exit=0），用时 519 s，`err.log` 为 0 字节。下表是 `out.json` 里它的全部工具调用。shell 调用只有那五条允许的命令，没有任何写操作：

| # | 工具 | 输入 |
|---|---|---|
| 1 | read | `docs/mac/SUBAGENT_BRIEF.md` |
| 2 | shell | `bash docs/mac/mac-steps.sh ci`（`timeout: 0`） |
| 3 | shell | `bash docs/mac/mac-steps.sh test-corpus`（`timeout: 0`） |
| 4 | shell | `bash docs/mac/mac-steps.sh regress`（`timeout: 0`） |
| 5 | shell | `bash docs/mac/mac-steps.sh compare`（`timeout: 0`） |
| 6 | shell | `bash docs/mac/mac-steps.sh details`（`timeout: 0`） |
| 7–11 | read | `target/mac`、`ci.log`、`test-corpus.log`、`mac-compare.txt`（30 行）、`compare.log` |
| 12–14 | grep | `regress.log` 中的 `^(regression\|xtask)`、汇总行、`^xtask: \d+ regressions` |
| 15–16 | read | `regress.log` 第 228–233 行、第 1–6 行 |

它的最终汇报原文（未改动，仅去掉绝对路径前缀）：

> All five commands have been run in order, each to completion. Report follows.
>
> ## 1. `ci`
>
> 最后一行：
>
> ```
> mac-steps ci: exit=1 elapsed=2s log=target/mac/ci.log
> ```
>
> `target/mac/ci.log` 最先出现的错误：
>
> ```
> error: couldn't read `crates/exec/tests/corpus/../support/mod.rs`: No such file or directory (os error 2)
>    --> crates/exec/tests/nodes_set_globals.rs:586:5
>     |
> 586 |     mod support;
>     |     ^^^^^^^^^^^^
>
> Error writing files: failed to resolve mod `support`: crates/exec/tests/corpus/../support/mod.rs does not exist
> xtask: cargo fmt --all --check failed (exit status: 1)
> ```
>
> ## 2. `test-corpus`
>
> 最后一行：
>
> ```
> mac-steps test-corpus: exit=1 elapsed=23s log=target/mac/test-corpus.log
> ```
>
> `target/mac/test-corpus.log` 最先出现的错误：
>
> ```
> error: couldn't read `crates/exec/tests/corpus/../support/mod.rs`: No such file or directory (os error 2)
>    --> crates/exec/tests/nodes_set_globals.rs:586:5
>     |
> 586 |     mod support;
>     |     ^^^^^^^^^^^^
>
> error: could not compile `dms-exec` (test "nodes_set_globals") due to 1 previous error
> warning: build failed, waiting for other jobs to finish...
> xtask: cargo test -p dms-strfmt -p dms-stream -p dms-sav -p dms-io -p dms-clem -p dms-pmml -p dms-algo -p dms-exec --features dms-strfmt/corpus,dms-stream/corpus,dms-sav/corpus,dms-io/corpus,dms-clem/corpus,dms-pmml/corpus,dms-algo/corpus,dms-exec/corpus failed (exit status: 101)
> ```
>
> ## 3. `regress`
>
> 最后一行：
>
> ```
> mac-steps regress: exit=1 elapsed=401s log=target/mac/regress.log
> ```
>
> `target/mac/regress.log` 最先出现的错误（第 229–230 行）：
>
> ```
> regression  pmml_import/pmml_cart_regression: 0/1 terminal inputs match Modeler
>   id1GR1197KPE27 (importedmodel): not produced node id1GR1197KPE27: upstream node id1AR111PJTLYQ failed
> ```
>
> 汇总行（regress.log 第 378–379 行）：
>
> ```
> xtask regress: 1596 streams: 1451 match, 91 statistical, 41 known GAPs, 13 regressions
> xtask: 13 regressions
> ```
>
> ## 4. `compare`
>
> 最后一行：
>
> ```
> mac-steps compare: exit=1 elapsed=0s log=target/mac/compare.log
> ```
>
> `target/mac/mac-compare.txt` 前 30 行（文件共 21 行，全部原样）：
>
> ```
> windows streams: 1597, mac streams: 1596
>      known-gap -> known-gap    41
>          match -> absent       1   <-- differs
>          match -> match        1451
>          match -> regression   13   <-- differs
>    statistical -> statistical  91
>
> pmml_import/pmml_cart_regression: match -> regression  id1GR1197KPE27 (importedmodel): not produced node id1GR1197KPE27: upstream node id1AR111PJTLYQ failed
> pmml_import/pmml_quest_classification: match -> regression  idXD11HALQFLN (importedmodel): not produced node idXD11HALQFLN: upstream node id6TS1Z7KEQ6G3 failed
> sources/excel_named_range: match -> absent  
> xml_scan/xml_default: match -> regression  id5UZ11IR3IBH6 (xmlimport): not produced node id5UZ11IR3IBH6: I/O: cannot find data file "D:/data-mining-software/corpus/sources/source_records.xml"
> xml_scan/xml_directory: match -> regression  id56T11B5RTZ8I (xmlimport): not produced node id56T11B5RTZ8I: I/O: cannot find directory "D:/data-mining-software/corpus/sources/source_xml_dir"
> xml_scan/xml_directory_recursive: match -> regression  id86911L91YXPC (xmlimport): not produced node id86911L91YXPC: I/O: cannot find directory "D:/data-mining-software/corpus/sources/source_xml_dir"
> xml_scan/xml_dtd: match -> regression  id5CJ11JUMZQGL (xmlimport): not produced node id5CJ11JUMZQGL: I/O: cannot find data file "D:/data-mining-software/corpus/sources/source_schema.xml"
> xml_scan/xml_fields_absolute: match -> regression  id4831ZIMJNF5E (xmlimport): not produced node id4831ZIMJNF5E: I/O: cannot find data file "D:/data-mining-software/corpus/sources/source_records.xml"
> xml_scan/xml_fields_relative: match -> regression  id7IU1ZARHVM8L (xmlimport): not produced node id7IU1ZARHVM8L: I/O: cannot find data file "D:/data-mining-software/corpus/sources/source_records.xml"
> xml_scan/xml_nested_records: match -> regression  id1RG1ZSC86RVQ (xmlimport): not produced node id1RG1ZSC86RVQ: I/O: cannot find data file "D:/data-mining-software/corpus/sources/source_nested.xml"
> xml_scan/xml_record_path: match -> regression  id71811G672TKI (xmlimport): not produced node id71811G672TKI: I/O: cannot find data file "D:/data-mining-software/corpus/sources/source_records.xml"
> xml_scan/xml_repeated_elements: match -> regression  id3JJ11EY6KT44 (xmlimport): not produced node id3JJ11EY6KT44: I/O: cannot find data file "D:/data-mining-software/corpus/sources/source_repeated.xml"
> xml_scan/xml_text_whitespace: match -> regression  id6NU11NGDDCRG (xmlimport): not produced node id6NU11NGDDCRG: I/O: cannot find data file "D:/data-mining-software/corpus/sources/source_text.xml"
> xml_scan/xml_xsd: match -> regression  id29S1ZWF4FS41 (xmlimport): not produced node id29S1ZWF4FS41: I/O: cannot find data file "D:/data-mining-software/corpus/sources/source_schema.xml"
> ```
>
> ## 5. `details`
>
> 最后一行（退出码 0，无 `target/mac/details.log` 错误需复制）：
>
> ```
> mac-steps details: 13 streams compared elapsed=20s dir=target/mac/details
> ```
>
> ## `target/mac/` 下的文件名
>
> ```
> agent/
> details/
> ci.log
> compare.log
> details.log
> mac-compare.txt
> mac-regress.tsv
> regress.log
> test-corpus.log
> ```

协调者对照日志的核查结果见 `MAC_REPORT.md` 的“子代理的汇报与核查”：逐项一致。

## 3. 平台问题：内联模块里的 `#[path]`

第一轮 `target/mac-run1/ci.log` 的全文：

```
$ cargo fmt --all --check
error: couldn't read `crates/exec/tests/corpus/../support/mod.rs`: No such file or directory (os error 2)
   --> crates/exec/tests/nodes_set_globals.rs:586:5
    |
586 |     mod support;
    |     ^^^^^^^^^^^^

Error writing files: failed to resolve mod `support`: crates/exec/tests/corpus/../support/mod.rs does not exist
xtask: cargo fmt --all --check failed (exit status: 1)
mac-steps ci: exit=1 elapsed=2s log=target/mac/ci.log
```

第一轮 `test-corpus.log` 失败在同一处：`error: could not compile 'dms-exec' (test "nodes_set_globals")`。

POSIX 的路径解析要求 `..` 之前的每一级目录都真实存在，`tests/corpus/` 不存在：

```
$ ls crates/exec/tests/corpus/../support/mod.rs
ls: crates/exec/tests/corpus/../support/mod.rs: No such file or directory
$ ls crates/exec/tests/support/mod.rs
crates/exec/tests/support/mod.rs
```

仓库里只有这一处 `#[path]`：`grep -rn '#\[path' crates apps xtask` 只命中 `crates/exec/tests/nodes_set_globals.rs:585`。修复见提交 `dbb1738`。修复后 `cargo fmt --all --check` 和 clippy 都通过，这个测试在 `corpus` feature 下能编译。

## 4. 平台问题：跨标签的数据引用

### 第一轮的 13 个 regression

`target/mac-run1/regress.log`：

```
xtask regress: 1596 streams: 1451 match, 91 statistical, 41 known GAPs, 13 regressions
xtask: 13 regressions
```

`target/mac-run1/mac-compare.txt` 的汇总部分：

```
windows streams: 1597, mac streams: 1596
     known-gap -> known-gap    41
         match -> absent       1   <-- differs
         match -> match        1451
         match -> regression   13   <-- differs
   statistical -> statistical  91
```

每个流的根因：`dms-cli run` 的 `run.json` 中 `root_causes`。所有 13 个都是找不到文件，没有一个是数值不符。

| 流 | 根因（`dms-cli run` 原文） | 文件实际所在 |
|---|---|---|
| `pmml_import/pmml_cart_regression` | `cannot find data file "D:/data-mining-software/corpus/cart_ofs2/ids200.csv"` | `corpus/cart_ofs2/` |
| `pmml_import/pmml_quest_classification` | `cannot find data file "D:/data-mining-software/corpus/models_next/class_grid.csv"` | `corpus/models_next/` |
| `xml_scan/xml_default` | `cannot find data file "D:/data-mining-software/corpus/sources/source_records.xml"` | `corpus/sources/` |
| `xml_scan/xml_directory` | `cannot find directory "D:/data-mining-software/corpus/sources/source_xml_dir"` | `corpus/sources/` |
| `xml_scan/xml_directory_recursive` | `cannot find directory "D:/data-mining-software/corpus/sources/source_xml_dir"` | `corpus/sources/` |
| `xml_scan/xml_dtd` | `cannot find data file "D:/data-mining-software/corpus/sources/source_schema.xml"` | `corpus/sources/` |
| `xml_scan/xml_fields_absolute` | `cannot find data file "D:/data-mining-software/corpus/sources/source_records.xml"` | `corpus/sources/` |
| `xml_scan/xml_fields_relative` | `cannot find data file "D:/data-mining-software/corpus/sources/source_records.xml"` | `corpus/sources/` |
| `xml_scan/xml_nested_records` | `cannot find data file "D:/data-mining-software/corpus/sources/source_nested.xml"` | `corpus/sources/` |
| `xml_scan/xml_record_path` | `cannot find data file "D:/data-mining-software/corpus/sources/source_records.xml"` | `corpus/sources/` |
| `xml_scan/xml_repeated_elements` | `cannot find data file "D:/data-mining-software/corpus/sources/source_repeated.xml"` | `corpus/sources/` |
| `xml_scan/xml_text_whitespace` | `cannot find data file "D:/data-mining-software/corpus/sources/source_text.xml"` | `corpus/sources/` |
| `xml_scan/xml_xsd` | `cannot find data file "D:/data-mining-software/corpus/sources/source_schema.xml"` | `corpus/sources/` |

`xtask compare` 原来给 `dms-cli run` 的搜索目录（例如 `details/xml_scan__xml_default.txt`）：

```
--data-dir corpus/xml_scan --data-dir corpus/ibm-demo-data --data-dir /Users/mac/modeler-home/Demos
```

这三个目录都不含上表最后一列的文件，所以按文件名回退也找不到。

### 影响范围

1561 个语料流的保存路径里含有 `data-mining-software`，即录制机器上的仓库路径。其中绝大多数指向本标签目录，按文件名回退能找到，在 Mac 上也是 match。用 `dms-cli xml` 解码主文档后，引用**其他**标签目录的流只有：

```
excel_gui/*.str（15 个）           -> sources        （excel_gui 不在 regress 的标签里）
pmml_import/pmml_cart_regression     -> cart_ofs2, misc_next
pmml_import/pmml_quest_classification -> models_next, misc_next
```

`xml_scan` 的引用不在主文档里，而在扩展节点的 XmlText 附件（`data/0001.dat`）中。所以修复同时扫描主文档和文本附件。`misc_next` 的引用是 `D:\…\misc_next\pmml_template.str`（反斜杠），只是模板来源记录，按名字找不到也不影响运行。

### 修复后的同一批流

修复见提交 `8cdacc1`。修复后 `target/mac/mac-regress.tsv` 中这两个标签的全部流：

```
pmml_import/pmml_cart_regression      match  1/1 terminal inputs match Modeler
pmml_import/pmml_quest_classification match  1/1 terminal inputs match Modeler
xml_scan/xml_default                  match  1/1 terminal inputs match Modeler
xml_scan/xml_directory                match  1/1 terminal inputs match Modeler
xml_scan/xml_directory_recursive      match  1/1 terminal inputs match Modeler
xml_scan/xml_dtd                      match  1/1 terminal inputs match Modeler
xml_scan/xml_empty                    match  0/0 terminal inputs match Modeler
xml_scan/xml_fields_absolute          match  1/1 terminal inputs match Modeler
xml_scan/xml_fields_relative          match  1/1 terminal inputs match Modeler
xml_scan/xml_malformed_rejected       match  0/0 terminal inputs match Modeler
xml_scan/xml_namespace_rejected       match  0/0 terminal inputs match Modeler
xml_scan/xml_nested_records           match  1/1 terminal inputs match Modeler
xml_scan/xml_record_path              match  1/1 terminal inputs match Modeler
xml_scan/xml_repeated_elements        match  1/1 terminal inputs match Modeler
xml_scan/xml_text_whitespace          match  1/1 terminal inputs match Modeler
xml_scan/xml_xsd                      match  1/1 terminal inputs match Modeler
```

预期被拒绝的流（`xml_malformed_rejected`、`xml_namespace_rejected`）和空结果的流（`xml_empty`）分类没有变化，说明新增的搜索目录没有把应当失败的流变成能运行。

新增单元测试 `compare::tests::corpus_labels_in_saved_paths` 通过（`cargo test -p xtask corpus_labels`：1 passed）。

## 5. 修复后的回归与基线逐流对照

`target/mac/regress.log` 和各步的最后一行：

```
xtask regress: 1596 streams: 1464 match, 91 statistical, 41 known GAPs, 0 regressions
mac-steps regress: exit=0 elapsed=411s log=target/mac/regress.log
mac-steps compare: exit=1 elapsed=0s log=target/mac/compare.log
mac-steps details: 0 streams compared elapsed=0s dir=target/mac/details
```

`target/mac/mac-compare.txt` 全文：

```
windows streams: 1597, mac streams: 1596
     known-gap -> known-gap    41
         match -> absent       1   <-- differs
         match -> match        1464
   statistical -> statistical  91

sources/excel_named_range: match -> absent
```

`compare_baseline.py` 只比较分类。下面另外逐流比较了 `detail` 列：`docs/mac/windows-baseline.tsv` 对比 `target/mac/mac-regress.tsv`，按 (label, stem) 配对。

```
same class: 1596 detail equal: 1596 detail differs: 0
```

Windows 基线中 `sources/excel_named_range` 一行为 `match  0/0 terminal inputs match Modeler`。它在 Mac 上 absent，是因为数据包按约定不含 `oracle/sources/excel_named_range`。

## 6. time_series 单元测试：分叉点与 bisect

失败信息（`cargo test --workspace --no-fail-fast`，连续三次结果相同）：

```
thread 'nodes::time_series::tests::transfer_seasonal_factors_transform_difference_and_delay' panicked at crates/exec/src/nodes/time_series.rs:2456:17:
logarithmic: Forecast { point: 2.496180285714286, lower: -1.675746608817442, upper: 6.668107180246014 }, 0.43296428571428597
```

在测试里临时加了打印（已还原，未提交），对每个打分点输出引擎结果和测试独立递推的期望值。以下全部是我们自己的计算结果，不是 Modeler 的值。

```
max |ln(exp(z)) - z| over the 40 inputs = 0
t=10 point=2.496180285714286 expected=2.432964285714286 diff=6.3216e-2
t=11 point=2.567107485714285 expected=2.516664285714285 diff=5.0443e-2
t=12 point=2.637992925714285 expected=2.573904285714285 diff=6.4089e-2
t=13 point=2.709950013714284 expected=2.690652285714285 diff=1.9298e-2
t=14 point=2.781851431314287 expected=2.766651885714287 diff=1.5200e-2
t=15 point=2.853741714834286 expected=2.834501805714286 diff=1.9240e-2
t=16 point=2.925953771538286 expected=2.920161789714285 diff=5.7920e-3
t=19 point=3.142629881600877 expected=3.140892265746285 diff=1.7376e-3
t=22 point=3.359507714481384 expected=3.358986429554547 diff=5.2128e-4
t=25 point=3.576446064344419 expected=3.576289678865004 diff=1.5639e-4
t=28 point=3.793402569303324 expected=3.793355653659488 diff=4.6916e-5
t=31 point=4.010364520791004 expected=4.010350446097853 diff=1.4075e-5
t=34 point=4.227328106237284 expected=4.227323883829339 diff=4.2224e-6
t=37 point=4.444292181871197 expected=4.444290915148813 diff=1.2667e-6
t=39 point=4.588934983165956 expected=4.588933720811580 diff=1.2624e-6
```

解读：

- macOS 上 `ln(exp(z))` 与 z 逐位相等，数学库调用不是差异来源。
- 差值从第一个打分点 t=10 就是 0.063，之后每 3 步约乘以 0.3（季节分母），每 1 步约乘以 0.2（普通分母）。这是递推初值不同，不是舍入误差被放大。
- 其余计算只有加减乘除，IEEE 754 下跨平台逐位一致。

Bisect：在同一台 Mac 上，用临时 worktree 和独立的 `CARGO_TARGET_DIR`，运行 `cargo test -p dms-exec --lib transfer_seasonal_factors_transform_difference_and_delay`：

```
d2a93c5^ (979f397): test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 34 filtered out
d2a93c5:            test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 34 filtered out
```

`d2a93c5` 对 `crates/algo/src/transfer.rs` 的关键改动：

```diff
-            let prefix = lost + e.delay + e.numerator.len() - 1;
+            let prefix = lost + e.delay;
```

同时，早于第一个预测变量值的分子滞后改为取第一个值（`None => first`），响应因此从 delay 处开始递推。测试的独立递推（`time_series.rs` 第 2438–2447 行）仍是旧结构：用稳态初值 `gain * w[0] / 0.8` 和 `gain * w[0] / (0.8 * 0.7)` 填充，从 t=10（4 次差分、2 期延迟和分子阶数之后）才开始递推。两者在 t=10 时的递推状态不同，于是出现上面衰减的差值。这个失败与平台无关，Windows 的 d1d2836 上同样存在。

## 7. test-corpus 的其余失败

`cargo test -p dms-strfmt -p dms-stream -p dms-sav -p dms-io -p dms-clem -p dms-pmml -p dms-algo -p dms-exec --features <各 crate>/corpus --no-fail-fast`，用时 588 s：

```
error: 4 targets failed:
    `-p dms-clem --test oracle`
    `-p dms-exec --lib`
    `-p dms-exec --test database`
    `-p dms-io --test statistics_corpus`
```

汇总：675 passed，4 个测试失败。`database_oracles` 由父测试启动子进程运行，所以日志里出现两次 FAILED。

`dms-clem --test oracle`：

```
thread 'clem_matches_modeler_on_probe_expressions' panicked at crates/clem/tests/oracle.rs:153:82:
called `Result::unwrap()` on an `Err` value: Os { code: 2, kind: NotFound, message: "No such file or directory" }
```

第 153 行读取 `probe_dir().join("manifest.json")`，其中 `probe_dir()` = `oracle/probes/clem_semantics`。数据包里 `oracle/probes/` 下有 0 个文件（`tar -tzf … | grep -c '^oracle/probes'` → 0）。

`dms-io --test statistics_corpus`：

```
thread 'corpus_sav_files_match_modeler' panicked at crates/io/tests/statistics_corpus.rs:230:5:
only 0 files checked
```

这个测试只检查那些在 `oracle/probes/sav_to_csv/{base}.csv` 有对照文件的 `.sav`，然后断言 `count >= 30`。原因同上。

`dms-exec --test database`：

```
thread 'live::database_oracles' panicked at crates/exec/tests/database.rs:249:17:
database DSN dms_access required (64-bit Access driver/database access): node id3FD11ZNFSCIV: I/O: ODBC DSN connection failed: … ODBC operation failed in SQLConnect (SQLSTATE State([73, 77, 48, 48, 50]))
```

SQLSTATE 字节 `[73, 77, 48, 48, 50]` 即 `IM002`（找不到数据源名称）。Mac 上没有 Access ODBC 驱动和这个 DSN，属于 Windows 专用环境。

`dms-exec --lib` 就是第 6 节的单元测试。

`cargo test --workspace --no-fail-fast`（`ci` 的测试部分），用时 158 s：108 个测试二进制，573 passed，1 failed（同一个 time_series 测试）。`ci` 的其余部分：

```
xtask layers: 13 packages ok
xtask parity: docs/parity-checklist.md up to date
```

## 8. 复现命令

在 `mac-validation` 分支上，数据按 `MAC_VALIDATION.md` 第 2 步放好：

```sh
source "$HOME/.cargo/env"
bash docs/mac/mac-steps.sh regress && bash docs/mac/mac-steps.sh compare   # 0 regressions；只有 excel_named_range absent
cargo test -p xtask corpus_labels                                          # 新增的单元测试
cargo test --workspace --no-fail-fast                                      # 只剩 time_series 一个失败
git worktree add --detach ../dms-before d2a93c5^ && git worktree add --detach ../dms-after d2a93c5
(cd ../dms-before && cargo test -p dms-exec --lib transfer_seasonal_factors)   # 通过
(cd ../dms-after && cargo test -p dms-exec --lib transfer_seasonal_factors)    # 失败
git worktree remove ../dms-before && git worktree remove ../dms-after
```
