# @kurumi/config 形式化契约

> 版本：0.1.0  
> 范围：`packages/config/src`（schema 适配层、来源、合并、插值、loader、store、Node 兼容入口）。  
> 判定：配置是生产路径上的对外契约模块；正确性、安全、错误与资源边界相关条款按强制（MUST）处理。schema 核心由 `@kurumi/schema` 维护（见 `docs/spec/schema.md`），config 只保留配置语义适配。已引入 `fast-check` 属性测试覆盖合并不可变性/幂等性；并发仅涉及 store 的 single-flight 与状态流转，事务型并发与变异测试仍作为后续增量项。

## 1. 流水线

```text
Sources(并行加载，按声明顺序取错误)
  -> Merge(后者覆盖前者，对象递归、数组/标量替换)
  -> Interpolate(字符串值 ${VAR} / ${VAR:-fallback} / {env:VAR} / {env:VAR:-fallback})
  -> Validate(schema；聚合全部字段问题)
  -> Freeze(递归冻结)
  -> Config
```

来源优先级：`sources[0] < sources[1] < ... < sources[n-1]`。默认值由 schema 在 Validate 阶段填充，不参与来源合并。

## 2. 类型与前置约束

| 编号             | 前置约束                                                                                                                    | 失败语义                                                                           |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `FC-CFG-PRE-001` | `schema` 必须是 `@kurumi/schema` 的 `Schema` 实例；通用 schema 会经配置适配层转换，校验失败统一为 `ConfigValidationError`。 | `ConfigSchemaDefinitionError` / `ConfigValidationError`                            |
| `FC-CFG-PRE-002` | 来源名称非空且互不重复。                                                                                                    | `ConfigSchemaDefinitionError`                                                      |
| `FC-CFG-PRE-003` | 每个来源返回 `undefined` 或普通对象；不得返回数组/原始值。                                                                  | `ConfigSourceError('invalid_layer')`                                               |
| `FC-CFG-PRE-004` | 文件路径非空；`format` 只能是 `auto/json/jsonc`，可推断或显式提供 `parser`。                                                | 定义期 `ConfigSchemaDefinitionError`；加载期 `ConfigSourceError('unknown_format')` |
| `FC-CFG-PRE-005` | 环境变量前缀匹配 `[A-Za-z_][A-Za-z0-9_]*`；分隔符为非字母数字、非空白字符。                                                 | `ConfigSchemaDefinitionError`                                                      |
| `FC-CFG-PRE-006` | 字符串长度/数组长度/数值范围/键数量上下界必须有序且为非负整数或有限数。                                                     | `ConfigSchemaDefinitionError`                                                      |
| `FC-CFG-PRE-007` | `default(value)` 的 value 非 `undefined` 且立即通过当前 schema。                                                            | `ConfigSchemaDefinitionError`                                                      |
| `FC-CFG-PRE-008` | `union` 至少两个分支；`oneOf` 至少一个字面量；`literal`/`oneOf` 的 `shouldCoerceStrings` 缺省 true。                        | `ConfigSchemaDefinitionError`                                                      |
| `FC-CFG-PRE-009` | `isNonEmpty` 表示 trim 后非空；`shouldTrim=false` 时纯空白也必须拒绝。                                                      | `ConfigValidationError`（`too_small`）                                             |

## 3. 后置约束

| 编号              | 后置约束                                                                                                                                         |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `FC-CFG-POST-001` | 合并：对象递归；数组、标量、`null` 整体替换；`undefined` 不覆盖；不修改任何输入层。                                                              |
| `FC-CFG-POST-002` | 对象 section 缺失默认按 `{}` 解析，嵌套默认值生效，必填叶子报告完整路径；`missingSection: 'error'` 时按必填缺失拒绝。                            |
| `FC-CFG-POST-003` | 标量强转：number 接受严格十进制/科学计数法字符串；boolean 接受 `true/false/1/0`（大小写不敏感）；可用 `shouldCoerceStrings: false` 关闭。        |
| `FC-CFG-POST-004` | 环境变量：`prefix` 后按 `separator` 分段；默认小写；纯数字段为数组下标；结构化 `{...}`/`[...]` 以 JSON 解析。                                    |
| `FC-CFG-POST-005` | 插值：`${NAME}` 与 `{env:NAME}` 取环境变量；`:-fallback` 缺省回退；`$${NAME}` 输出字面量；仅处理字符串值；环境变量值不递归插值。                 |
| `FC-CFG-POST-006` | 校验失败必须一次性返回全部问题，路径形如 `$.server.port` / `$.hosts[0]`。                                                                        |
| `FC-CFG-POST-007` | 成功返回的配置递归冻结，嵌套对象/数组不可写。                                                                                                    |
| `FC-CFG-POST-008` | `secret()` 子树在 `redact()` 中替换为 `[REDACTED]`；联合类型使用首个可解析分支的遮蔽规则；`redact` 忽略危险键。                                  |
| `FC-CFG-POST-009` | 文件：`.json` 严格解析；`.jsonc` 支持注释、尾随逗号、BOM，注释等价于空白且不得合并相邻 token；根必须是对象；语法错误的 `sourceName` 为来源名称。 |
| `FC-CFG-POST-010` | store 仅在配置结构变化时广播；`equals` 可覆盖比较策略；`equals` 抛错包装为 `compare_error` 并保留旧配置。                                        |
| `FC-CFG-POST-011` | store reload 失败时保留上一份可用配置，记录 `lastError`，状态为 `error`；后续成功恢复 `ready`。                                                  |
| `FC-CFG-POST-012` | 并发 `reload()` 复用同一 Promise，同一时刻最多一次加载；`signal` 只对首次发起的加载生效。                                                        |
| `FC-CFG-POST-013` | 配置结果类型为 readonly；运行时递归冻结，禁止写入。                                                                                              |
| `FC-CFG-POST-014` | 插值、合并、redact 对危险键显式失败或跳过，不得替换对象原型。                                                                                    |

## 4. 系统不变量

| 编号        | 不变量                                                                                                                  |
| ----------- | ----------------------------------------------------------------------------------------------------------------------- |
| `I-CFG-001` | 任意来源、任意层级都不得写入 `__proto__` / `prototype` / `constructor`。                                                |
| `I-CFG-002` | 配置嵌套深度 `<= 64`；包括环境变量结构化 JSON，超限显式失败。                                                           |
| `I-CFG-003` | 合并、插值、校验、redact 不得修改输入对象。                                                                             |
| `I-CFG-004` | store 的 `get()` 在初始加载成功后始终返回一份完整配置，即使最近一次 reload 失败。                                       |
| `I-CFG-005` | `dispose()` 后不再广播；`subscribe`/`reload` 必须显式拒绝。                                                             |
| `I-CFG-006` | 核心 config 入口不得导入 `node:`；Node 能力仅通过 `@kurumi/config/node`。                                               |
| `I-CFG-007` | 所有失败必须抛出结构化错误并携带定位上下文，禁止静默吞错；取消必须保留 `AbortError`，不得包装为来源错误。               |
| `I-CFG-008` | config 核心不得直接输出日志、调用 `console.*` 或依赖 `@kurumi/log`；日志级别解析与输出由调用方通过 `@kurumi/log` 负责。 |

## 5. Store 状态机

```text
M = (States, Events, Transition_Function, Initial_State, Final_States)
States  = { Idle, Loading, Ready, Error, Disposed }
Events  = { Initialize, LoadSucceeded, LoadFailed, Reload, Dispose }
Initial = Idle
Final   = { Disposed }
```

| 当前状态          | 事件                    | 下一状态   | 副作用 / 拦截                                               |
| ----------------- | ----------------------- | ---------- | ----------------------------------------------------------- |
| `Idle`            | `Initialize`            | `Loading`  | 开始首次加载                                                |
| `Loading`         | `LoadSucceeded`（首次） | `Ready`    | 设置 current；工厂返回 store                                |
| `Loading`         | `LoadFailed`（首次）    | `Error`    | 记录 `lastError`；工厂 reject，不返回 store                 |
| `Ready`           | `Reload`                | `Loading`  | 保留上一份 current                                          |
| `Error`           | `Reload`                | `Loading`  | 保留上一份 current                                          |
| `Loading`         | `LoadSucceeded`         | `Ready`    | 更新 current；若结构变化且存在 listeners，广播              |
| `Loading`         | `LoadFailed`            | `Error`    | 保留上一份 current；reject 并抛出结构化错误                 |
| `Ready` / `Error` | `Dispose`               | `Disposed` | 清空 listeners；current 仍可读                              |
| `Loading`         | `Dispose`               | `Disposed` | 允许在飞行中 dispose；加载完成后不广播，状态保持 `Disposed` |
| `Disposed`        | `Reload` / `Subscribe`  | `Disposed` | 显式拒绝 `ConfigDisposedError`                              |
| `Disposed`        | `Dispose`               | `Disposed` | 幂等                                                        |

非法事件：`Idle` 上的 `Reload`、`Disposed` 上的 `LoadSucceeded` 广播均被拦截（不产生副作用）。

## 6. 异常分类

| 错误类型 / code                                                        | 触发条件                                                                                                           | 关键上下文                                         |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------- |
| `ConfigValidationError` / `validation_error`                           | schema 校验失败                                                                                                    | `issues[]`：`code`、`path`、`expected`、`received` |
| `ConfigSourceError` / `source_error`                                   | 来源读取/解析失败，或返回非法结构；细分 code 包括 `invalid_layer`、`invalid_root`、`parse_error`、`unknown_format` | `sourceName`、`cause`                              |
| `ConfigSyntaxError` / `syntax_error`                                   | JSON/JSONC 语法错误                                                                                                | `sourceName`（来源名称）、`offset`、`cause`        |
| `ConfigEnvMappingError` / `env_mapping_error`                          | 环境变量路径冲突、空段、数组断档、危险键、重复映射、结构化值超深                                                   | `issues[]`                                         |
| `ConfigInterpolationError` / `interpolation_error`                     | 缺失变量且无默认值、危险键                                                                                         | `issues[]`，路径指向字符串值                       |
| `ConfigError` / `compare_error`                                        | store 的 `equals` 回调抛错                                                                                         | `cause`                                            |
| `ConfigError` / `load_error`                                           | store 加载阶段抛出非 ConfigError                                                                                   | `cause`                                            |
| `ConfigSchemaDefinitionError` / `schema_definition_error`              | 非法 schema 选项、非法默认值、非法来源名称、非法文件 format                                                        | 直接消息                                           |
| `ConfigDisposedError` / `disposed`                                     | 对已销毁 store 执行 reload/subscribe                                                                               | 无                                                 |
| `KurumiError` / `invalid_value`、`forbidden_key`、`max_depth_exceeded` | 底层 JSON 值约束                                                                                                   | 路径标签                                           |

边界采样：

- 空字符串、纯空白、Unicode/emoji、超长字符串（由长度约束拒绝）。
- 数值 `0`、`-0`、`±1`、`MAX_SAFE_INTEGER`、`NaN`、`±Infinity`。
- 空数组、单元素、超上限数组；空对象、单键、超上限记录。
- 数组下标 `0`、`10000`、`10001`。
- 文件不存在、空文件、非法 JSON、根为数组、未知扩展名。
- 插值缺失、空环境变量、未闭合占位符、非法变量名、`$${}` 转义、`{env:}` 形式与混合写法。

## 7. 追溯矩阵

| 约束                                              | 测试文件 / 用例                                                                                                                                                                                 |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PRE-001`                                         | `src/loader.test.ts` · 拒绝非 Schema 实例；通用 schema 校验失败转换为 `ConfigValidationError`                                                                                                   |
| `POST-009` JSONC 注释                             | `src/parser/jsonc.test.ts` · 块注释按空白处理、不合并相邻 token                                                                                                                                 |
| `I-CFG-007` 取消                                  | `src/source/file-source.test.ts` · 取消保持 `AbortError`                                                                                                                                        |
| `PRE-002`                                         | `src/loader.test.ts` · 拒绝重复来源名称与空名称                                                                                                                                                 |
| `PRE-003`                                         | `src/loader.test.ts` · 来源返回非对象；undefined 来源被跳过                                                                                                                                     |
| `PRE-004` / `POST-009`                            | `src/source/file-source.test.ts` · 格式推断、非法 format、语法错误 sourceName                                                                                                                   |
| `PRE-005` / `POST-004`                            | `src/source/env-source.test.ts` · 前缀/分隔符/数组下标/结构化值                                                                                                                                 |
| `PRE-006` / `PRE-007` / `PRE-008` / `PRE-009`     | `src/schema/scalars.test.ts` · 拒绝非法选项 / default 立即校验 / `isNonEmpty`；`src/schema/composites.test.ts` · 要求至少两个分支；`@kurumi/schema` 的 `docs/spec/schema.md` 覆盖通用定义期约束 |
| `POST-001`                                        | `src/merge.test.ts`                                                                                                                                                                             |
| `POST-002`                                        | `src/schema/composites.test.ts` · 填充默认值、保留 optional 键、`missingSection`；`src/loader.test.ts` · 没有任何来源时报告必填字段                                                             |
| `POST-003`                                        | `src/schema/scalars.test.ts` · number/boolean 强转                                                                                                                                              |
| `POST-005`                                        | `src/interpolate.test.ts`                                                                                                                                                                       |
| `POST-006`                                        | `src/schema/composites.test.ts` · 聚合多个字段错误并给出完整路径                                                                                                                                |
| `POST-007`                                        | `src/loader.test.ts` · 返回结果被递归冻结                                                                                                                                                       |
| `POST-008` / `POST-014`                           | `src/schema/composites.test.ts` · `redact` 与危险键；`src/interpolate.test.ts` · 危险键                                                                                                         |
| `POST-010` / `POST-011` / `POST-012` / `POST-013` | `src/store.test.ts` · 广播、失败恢复、single-flight、equals、异步 listener；`src/schema/composites.test.ts` · readonly 类型                                                                     |
| `I-CFG-001`                                       | `src/merge.test.ts` · 拒绝原型污染键；`src/schema/composites.test.ts` · 拒绝原型污染键；`src/loader.test.ts` · 合并阶段的原型污染错误；`src/source/env-source.test.ts` · 结构化值危险键         |
| `I-CFG-002`                                       | `src/merge.test.ts` · 超过深度上限；`src/interpolate.test.ts` · 超过深度上限；`src/schema/composites.test.ts` · 深度防护；`src/source/env-source.test.ts` · 结构化值深度                        |
| `I-CFG-003`                                       | `src/merge.test.ts` · 不修改任何输入层；`src/interpolate.test.ts` · 不修改输入层                                                                                                                |
| `I-CFG-004`                                       | `src/store.test.ts` · reload 失败时保留上一份配置                                                                                                                                               |
| `I-CFG-005`                                       | `src/store.test.ts` · 销毁后状态为 Disposed                                                                                                                                                     |
| `I-CFG-006` / `I-CFG-008`                         | `src/architecture.test.ts`                                                                                                                                                                      |
| `I-CFG-007`                                       | `src/loader.test.ts` · 来源异常按声明顺序返回、AbortSignal；`src/source/file-source.test.ts` · 解析异常包装；`src/store.test.ts` · equals/异步 listener 错误包装                                |
| 状态机 `STA-*`                                    | `src/store.test.ts` · 初始加载、reload 广播、失败恢复、并发 single-flight、dispose                                                                                                              |
| 错误 `ERR-*`                                      | `src/parser/jsonc.test.ts`、`src/source/env-source.test.ts`、`src/source/file-source.test.ts`、`src/schema/scalars.test.ts`                                                                     |
| `PBT-CFG-001`                                     | `src/properties.test.ts` · 合并不可变性与幂等性                                                                                                                                                 |
| `PBT-CFG-002`                                     | `src/properties.test.ts` · 插值的确定性与不修改输入                                                                                                                                             |

## 8. 兼容性与演进

- 破坏性变更：调整合并语义、对象缺失语义、强转规则、默认值时机、错误码、`RuntimeHost` 必选成员、store 状态含义；把 `InferShape` 从 readonly 改回可变也属于类型契约破坏。
- 布尔选项已统一为 `should*` / `is*` 前缀：`shouldTrim`、`isNonEmpty`、`isInteger`、`shouldCoerceStrings`、`shouldLowercase`、`shouldParseStructuredValues`、`isOptional`、`shouldAllowMissing`；旧名称不再兼容。
- 非破坏性变更：新增 schema 选项且提供缺省值、新增来源类型、新增可选 store 方法、`reload(signal?)` 与 `loadConfig({ signal? })`、`ConfigChangeListener` 支持异步返回值。
- config 不拥有日志器、日志级别枚举或输出格式；相关语义由 `@kurumi/log` 契约维护，config 只产出可被 `parseLogLevel()` 解析的字符串。
- 修改公开 API（参数/返回值/抛错）必须同步更新本契约、TSDoc、调用点与测试，并在提交信息标注 `!`。
- 契约与代码不一致时以本文件为唯一真实源，先更新契约再改实现。
