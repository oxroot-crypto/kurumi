# @kurumi/log 形式化契约

> 版本：0.1.0  
> 范围：`packages/log/src`（日志级别、结构化记录、Logger、sink、格式化与 Node 文件 sink）。  
> 判定：日志器是所有包共享的对外契约类基础设施，且处于生产调用路径；正确性、错误语义、资源释放与跨 runtime 隔离按强制（MUST）处理。已引入 `fast-check` 属性测试覆盖字段归一化边界；异步 sink 写入队列与变异测试仍作为后续增量项。

## 1. 适用范围与豁免判定

`@kurumi/log` 不承载业务数据，但决定所有上层服务的可观测性、错误上报与资源生命周期，属于 FSVDD §1.2 中“对外接口 / 数据处理”模块，**不适用**纯文档或构建配置豁免。持久化契约采用本文件。

日志器不负责：

- 从配置文件加载日志级别（由应用调用 `parseLogLevel()` 完成）；
- 采集指标、链路追踪或网络上报（可通过自定义 `LogSink` 扩展）；
- 修改 `process.env`、全局 console 或捕获未处理异常。

## 2. 类型与前置约束

```text
LogLevel        = trace | debug | info | warn | error | fatal | silent
EmittedLogLevel = LogLevel \ { silent }
LogFields       = { [key: string]: unknown }
LogValue        = JsonValue
LogRecord       = { timeMs, level, message, bindings, fields? }
LogSink         = { name?, write(record), flush?(), close?() }
Logger          = { level, log, trace..fatal, child, withLevel, isLevelEnabled, flush, close }
```

### 2.1 前置约束

| 编号             | 前置约束                                                                                                                                                        | 失败语义                                                 |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- |
| `FC-LOG-PRE-001` | `level` 必须是合法 `LogLevel`；`silent` 只能作为阈值，不能作为写入级别。                                                                                        | `LogDefinitionError('log_definition_error')`             |
| `FC-LOG-PRE-002` | `maxDepth`、`maxStringLength`、`maxEntries`、`maxMessageLength` 必须是满足定义域的安全整数（深度允许 0，其余必须 >= 1）。                                       | `LogDefinitionError`                                     |
| `FC-LOG-PRE-003` | 每个 sink 必须实现 `write(record)`；`sinks` 允许为空数组（显式丢弃日志）。                                                                                      | `LogDefinitionError`                                     |
| `FC-LOG-PRE-004` | `clock` 必须是函数，且每次调用返回有限数值（毫秒时间戳）。                                                                                                      | 构造期 `LogDefinitionError`；调用期 `LogDefinitionError` |
| `FC-LOG-PRE-005` | `onSinkError` 若提供必须为函数，只接收 `write` 失败，可同步或异步；同步抛出/异步 rejection 都视为新的 sink 失败。`flush()`/`close()` 的 sink 失败总是聚合抛出。 | `LogSinkError`                                           |
| `FC-LOG-PRE-006` | `message` 必须为字符串；运行期收到非字符串时按 `String(message)` 转换，保证诊断入口不因类型错误中断。                                                           | 无（防御式转换）                                         |
| `FC-LOG-PRE-007` | `parseLogLevel` 只接受大小写不敏感的 `trace/debug/info/warn/error/fatal/silent`，其他输入返回 `undefined`。                                                     | 返回 `undefined`，不抛错                                 |

### 2.2 后置约束

| 编号              | 后置约束                                                                                                                                                                                      |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FC-LOG-POST-001` | 阈值过滤：仅当 `weight(record.level) >= weight(logger.level)` 且阈值不是 `silent` 时写入；关闭级别不构造记录、不调用 sink。                                                                   |
| `FC-LOG-POST-002` | 记录形状固定为 `{ timeMs, level, message, bindings, fields? }`；`timeMs` 来自注入时钟（毫秒），`level` 为写入级别。                                                                           |
| `FC-LOG-POST-003` | 记录、bindings 与 fields 递归冻结；每个 sink 收到同一个不可变记录，且 sink 调用顺序与 `sinks` 声明顺序一致；顶层用户字段数量受 `maxEntries` 限制，超出时额外产生一个 `[truncated]` 元数据键。 |
| `FC-LOG-POST-004` | fields 中的 `Error` 递归序列化为 `{ name, message, stack?, cause?, code?, ... }`；循环引用标记为 `[Circular]`，超深标记为 `[MaxDepth]`，危险键被丢弃。                                        |
| `FC-LOG-POST-005` | `child(bindings)` 生成新 logger；bindings 浅合并，子级覆盖父级；父 logger 的 bindings 不变。                                                                                                  |
| `FC-LOG-POST-006` | `withLevel(level)` 仅替换阈值，保留 bindings、sinks、时钟与错误策略。                                                                                                                         |
| `FC-LOG-POST-007` | 同步 sink 失败：所有 sink 仍会被尝试；无 handler 时按失败数量抛出 `LogSinkError` 或 `AggregateError`；有 handler 时逐个上报且不中断后续 sink；异步 handler 的 rejection 进入错误队列。        |
| `FC-LOG-POST-008` | 异步 sink 失败：立即上报 handler；无 handler 时记录到内部错误队列，由 `flush()`/`close()` 以 `AggregateError` 显式抛出，禁止静默吞错。                                                        |
| `FC-LOG-POST-009` | `flush()` 循环等待直到没有在飞行写入（包括 flush 期间新增的写入）；`close()` 先 flush，再按声明顺序关闭 sink；并发 `close()` 复用同一个 Promise，幂等且可重复调用。                           |
| `FC-LOG-POST-010` | `close()` 后调用任何写入方法抛 `LogClosedError`；`flush()`/`close()` 仍可安全调用。                                                                                                           |
| `FC-LOG-POST-011` | 默认 `level = info`，默认 sink 为 console sink；console sink 输出合法的人类可读文本或单行 JSON。                                                                                              |
| `FC-LOG-POST-012` | `serializeError()` 对任意错误对象保留 `name`、`message`，默认保留 `stack` 与 `cause`；非 Error 输入以 `NonError` 包装。                                                                       |
| `FC-LOG-POST-013` | `message` 受 `maxMessageLength`（缺省等于 `maxStringLength`）限制，截断标记也计入长度上限；字段 provider 抛错时降级为 `[fields_error]` 字段，不反向中断业务。                                 |
| `FC-LOG-POST-014` | Node 文件 sink 的 `write`/`flush`/`close` 失败统一为 `LogSinkError`；未观察的 `write()` 失败会在后续 `flush()`/`close()` 中显式抛出；成功写入会清除旧错误；并发 `close()` 复用同一 Promise。  |

### 2.3 系统不变量

| 编号        | 不变量                                                                                                                               |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `I-LOG-001` | 主入口 `@kurumi/log` 及其非 `node/` 源码不得导入 `node:` 内置模块；Node 文件 sink 只出现在 `@kurumi/log/node`。                      |
| `I-LOG-002` | Logger 不修改调用方传入的 bindings、fields、message 或 Error 对象。                                                                  |
| `I-LOG-003` | 记录一旦交给 sink 即不可变；任何 sink 都不能通过记录引用修改其他 sink 看到的内容。                                                   |
| `I-LOG-004` | 任意深度、循环引用、原型污染键、非有限数值、BigInt、Symbol、函数和 Date 都必须被转换为 JSON 安全值或显式标记，序列化过程不得栈溢出。 |
| `I-LOG-005` | sink 失败不得被静默吞掉：要么交给 `onSinkError`，要么在 `flush()`/`close()` 中显式抛出。                                             |
| `I-LOG-006` | `close()` 后不得再向任何 sink 写入；关闭操作幂等，sink 句柄最多释放一次。                                                            |
| `I-LOG-007` | 日志器不得读取配置包内部状态，也不得要求 `@kurumi/config` 作为依赖；级别字符串由 `parseLogLevel()` 解耦。                            |

## 3. 生命周期状态机

```text
M = (States, Events, Transition_Function, Initial_State, Final_States)
States  = { Open, Closing, Closed }
Events  = { Log, Flush, Close, FlushDone, CloseDone }
Initial = Open
Final   = { Closed }
```

| 当前状态  | 事件              | 下一状态  | 副作用 / 拦截                      |
| --------- | ----------------- | --------- | ---------------------------------- |
| `Open`    | `Log`             | `Open`    | 过滤后写入全部 sink                |
| `Open`    | `Flush`           | `Open`    | 等待在飞行写入；无未处理失败则完成 |
| `Open`    | `Close`           | `Closing` | 停止接收新日志，等待 flush         |
| `Closing` | `Log`             | `Closing` | 抛 `LogClosedError`                |
| `Closing` | `FlushDone`       | `Closing` | 继续关闭 sink                      |
| `Closing` | `CloseDone`       | `Closed`  | 句柄释放完成                       |
| `Closed`  | `Log`             | `Closed`  | 抛 `LogClosedError`                |
| `Closed`  | `Flush` / `Close` | `Closed`  | 幂等：flush 完成，close 直接返回   |

非法事件：`Closed` 上的 `Log` 必须显式拒绝；`Close` 重复调用不得重复关闭 sink。

## 4. 异常分类

| 错误类型 / code                               | 触发条件                                                                 | 关键上下文                        |
| --------------------------------------------- | ------------------------------------------------------------------------ | --------------------------------- |
| `LogDefinitionError` / `log_definition_error` | 非法级别、非法数值选项、非法 sink、非法时钟                              | 直接消息                          |
| `LogSinkError` / `log_sink_error`             | sink 的 `write`/`flush`/`close` 抛错，或 `onSinkError` 抛错              | `sinkName`、`cause`               |
| `LogClosedError` / `log_closed`               | 关闭后继续写日志                                                         | 无                                |
| `AggregateError`                              | `flush()` / `close()` 聚合的 sink 失败，或同步写入时有多个 sink 同时失败 | `errors[]`，元素为 `LogSinkError` |

边界采样：

- 级别：`silent` 阈值、最低 `trace`、最高 `fatal`、非法大小写/空串/未知字符串。
- 字段：`undefined`、`null`、`NaN`、`±Infinity`、`-0`、`BigInt`、`Symbol`、函数、Date、RegExp、循环对象、超深对象、超长字符串、`__proto__` 键。
- sink：空数组、同步抛错、异步 reject、flush 抛错、close 抛错、重复 close。
- 生命周期：在飞行 flush 时 close、关闭后 log/flush/close。

## 5. 追溯矩阵

| 约束                                   | 测试文件 / 用例                                                                                         |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `PRE-001` / `POST-001` / `PRE-007`     | `src/level.test.ts` · 级别顺序、过滤、解析与非法输入                                                    |
| `PRE-002` / `PRE-003` / `PRE-004`      | `src/logger.test.ts` · 构造期拒绝非法选项                                                               |
| `POST-002` / `POST-003`                | `src/logger.test.ts` · 记录形状、冻结与 sink 调用顺序                                                   |
| `POST-004` / `I-LOG-003` / `I-LOG-004` | `src/record.test.ts` · Error/循环/深度/危险键/特殊值归一化                                              |
| `POST-005` / `POST-006`                | `src/logger.test.ts` · child 合并、withLevel 保留配置                                                   |
| `POST-007` / `I-LOG-005`               | `src/logger.test.ts` · 同步/异步 sink 失败、同步/异步 handler                                           |
| `POST-008` / `POST-009` / `I-LOG-006`  | `src/logger.test.ts` · flush barrier、在飞行关闭、并发 close、幂等                                      |
| `POST-010` / 状态机                    | `src/logger.test.ts` · 关闭后写入抛错、重复关闭不重复释放                                               |
| `POST-011`                             | `src/sink/console.test.ts` · 默认级别与两种格式                                                         |
| `POST-012`                             | `packages/core/src/serialize.test.ts` · 错误链、非 Error、循环 cause、AggregateError                    |
| `POST-013`                             | `src/logger.test.ts` · maxMessageLength、顶层 maxEntries、字段 provider 降级                            |
| `POST-014`                             | `src/node/file-sink.test.ts` · 直接 flush 暴露写入错误、错误包装为 LogSinkError、成功重试后不残留旧错误 |
| `I-LOG-001`                            | `src/architecture.test.ts` · 主入口/非 node 源码无 `node:` 导入                                         |
| `I-LOG-007`                            | `src/architecture.test.ts` · 不依赖 `@kurumi/config`；config 端无 logger 调用                           |
| `PRE-005`                              | `src/logger.test.ts` · handler 抛出/异步 rejection 被包装                                               |
| `PBT-LOG-001`                          | `src/properties.test.ts` · 任意 JSON 字段归一化、条目上限与冻结                                         |

## 6. 兼容性与演进

- 破坏性变更：调整 `LogLevel` 名称/权重、`LogRecord` 必选字段、`LogSink.write` 签名、`Logger` 方法语义、默认级别或默认 sink。
- `LogRecord.time` 已重命名为 `timeMs`；`NodeFileSinkOptions.append` / `ensureDirectory` 已重命名为 `shouldAppend` / `shouldEnsureDirectory`；`RecordNormalizationOptions.includeStack` 已重命名为 `shouldIncludeStack`。以上均为破坏性变更。
- 非破坏性变更：新增可选 `LoggerOptions` 字段（如 `maxMessageLength`）、新增内置 sink、新增格式化格式且提供缺省值、`onSinkError` 支持异步返回值。
- 修改公开 API（参数/返回值/抛错）必须同步更新本契约、TSDoc、调用点与测试，并在提交信息标注 `!`。
- 契约与代码不一致时以本文件为唯一真实源，先更新契约再改实现。
