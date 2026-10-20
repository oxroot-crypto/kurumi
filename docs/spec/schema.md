# @kurumi/schema 形式化契约

> 版本：0.1.0  
> 范围：`packages/schema/src`（schema 基类、标量/组合 schema、问题模型、错误模型、JSON Schema 导出）。  
> 判定：schema 是 config 与 agent 工具输入共同依赖的对外契约类基础设施，涉及不可信输入、原型污染与错误语义，**不适用** FSVDD §1.2 的纯文档/构建配置豁免。持久化契约采用本文件。

## 1. 职责与边界

`@kurumi/schema` 负责：

- 定义跨 runtime 的不可变 schema 描述与链式包装器；
- 对不可信输入执行类型、范围、长度、字面量、联合与嵌套深度校验；
- 聚合全部字段级问题并输出稳定 `code` / `path`；
- 提供 `secret()` 遮蔽语义与 JSON Schema 导出，供工具定义复用；
- 保证解析不修改输入、默认值深拷贝、导出结果 JSON 安全。

不负责：

- 配置来源加载、合并、插值与 store（`@kurumi/config`）；
- Agent 循环、工具执行与模型适配（`@kurumi/agent`）；
- 日志、console 输出、Node/浏览器 API；
- JSON Schema 全量 2020-12 规范（只输出本包可表达的稳定子集）。

## 2. 核心类型

```text
SchemaKind = string | number | boolean | literal | array | object | record | union | optional | nullable | default
SchemaIssue = { code, path, message, expected?, received? }
ParseOutcome<T> = { ok: true, value: T } | { ok: false, issues: SchemaIssue[] }
Schema<T> = { kind, metadata, parse, parseOrThrow, optional, nullable, default, secret, describe, redact, toJsonSchema }
```

## 3. 系统不变量

| 编号           | 不变量                                                                                                           |
| -------------- | ---------------------------------------------------------------------------------------------------------------- |
| `I-SCHEMA-001` | 任意 schema 解析都不得修改输入对象；成功值由 schema 重新构造（`passthrough` 未知键也必须克隆且为 JSON 安全值）。 |
| `I-SCHEMA-002` | schema 构造后修改外部 `shape` / 默认值对象，不得改变已构造 schema 的行为。                                       |
| `I-SCHEMA-003` | 拒绝 `__proto__` / `prototype` / `constructor`；不得污染 `Object.prototype`。                                    |
| `I-SCHEMA-004` | 嵌套深度 `<= 64`；超限必须显式返回 `max_depth_exceeded`。                                                        |
| `I-SCHEMA-005` | 失败必须返回或抛出结构化错误并携带路径；禁止静默吞错。                                                           |
| `I-SCHEMA-006` | `toJsonSchema()` 输出必须是 JSON 安全值并递归冻结，重复调用结果结构相等。                                        |
| `I-SCHEMA-007` | 核心源码不得导入 `node:`、`@kurumi/agent` / `config` / `log`，不得调用 `console.*`。                             |
| `I-SCHEMA-008` | `secret()` 子树在 `redact()` 中整体替换为 `[REDACTED]`；联合类型使用首个可解析分支的遮蔽规则。                   |

## 4. 前置约束

| 编号                | 前置约束                                                                                         | 失败语义                           |
| ------------------- | ------------------------------------------------------------------------------------------------ | ---------------------------------- |
| `FC-SCHEMA-PRE-001` | `minLength` / `maxLength` / `minKeys` / `maxKeys` 必须是非负整数；`min` / `max` 必须是有限数值。 | `SchemaDefinitionError`            |
| `FC-SCHEMA-PRE-002` | 所有上下界必须有序：`min <= max`、`minLength <= maxLength`。                                     | `SchemaDefinitionError`            |
| `FC-SCHEMA-PRE-003` | `default(value)` 的 value 非 `undefined`，必须立即通过当前 schema 且为 JSON 安全值。             | `SchemaDefinitionError`            |
| `FC-SCHEMA-PRE-004` | `union()` 至少两个分支；`oneOf()` 至少一个字面量；字面量必须是有限数值或原始值。                 | `SchemaDefinitionError`            |
| `FC-SCHEMA-PRE-005` | `object()` 字段名不得是危险键；`unknownKeys` / `missingSection` 只能是枚举值。                   | `SchemaDefinitionError`            |
| `FC-SCHEMA-PRE-006` | `describe()` 文本 trim 后非空；`parseFailure()` 至少一条问题。                                   | `SchemaDefinitionError`            |
| `FC-SCHEMA-PRE-007` | 布尔选项必须为布尔值；正则约束内部移除 `g` / `y` 标志，避免 `lastIndex` 漂移。                   | `SchemaDefinitionError`            |
| `FC-SCHEMA-PRE-008` | 带强转的 number/boolean/literal 只接受严格十进制/科学计数法或 `true/false/1/0` 字符串。          | `invalid_type` / `invalid_literal` |
| `FC-SCHEMA-PRE-009` | `object` shape、options 与组合 schema 子项必须在构造期校验；非法结构抛 `SchemaDefinitionError`。 | `SchemaDefinitionError`            |

## 5. 后置约束

| 编号                 | 后置约束                                                                                                                                                             |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FC-SCHEMA-POST-001` | 标量：string 按 Unicode 码点计长；number 拒绝 NaN，默认拒绝 ±Infinity；boolean 默认只接受布尔值。                                                                    |
| `FC-SCHEMA-POST-002` | `shouldCoerceStrings` 显式开启后，number 接受严格数字字符串，boolean 接受 `true/false/1/0`（大小写不敏感）。                                                         |
| `FC-SCHEMA-POST-003` | object 缺省拒绝未知键；`strip` 丢弃；`passthrough` 保留未知键，但必须克隆为 JSON 安全值并拒绝函数/Date 等非法值。                                                    |
| `FC-SCHEMA-POST-004` | object `missingSection` 缺省 `error`；`empty` 时按 `{}` 解析，使嵌套默认值生效。                                                                                     |
| `FC-SCHEMA-POST-005` | array 拒绝稀疏数组，按下标报告路径；record 支持键正则与键数量边界。                                                                                                  |
| `FC-SCHEMA-POST-006` | union 按声明顺序返回首个匹配分支；全部失败时返回汇总问题与最接近分支的问题。                                                                                         |
| `FC-SCHEMA-POST-007` | 解析失败一次性聚合全部字段问题，路径形如 `$.server.port` / `$.hosts[0]`。                                                                                            |
| `FC-SCHEMA-POST-008` | `optional()` 允许 `undefined`；`nullable()` 仅额外允许 `null`，但其 `_acceptsUndefined` 必须委托内层 `optional`/`default`；`default()` 在缺失时填入深拷贝默认值。    |
| `FC-SCHEMA-POST-009` | `toJsonSchema()`：optional/default 从 `required` 排除；object/record/array 边界映射到 JSON Schema 关键字。                                                           |
| `FC-SCHEMA-POST-010` | `toJsonSchema()` 写入 `description`；`secret()` 不写入 JSON Schema。                                                                                                 |
| `FC-SCHEMA-POST-011` | `parseOrThrow()` 失败时抛 `SchemaValidationError`，`issues` 与 `parse()` 失败结果一致。                                                                              |
| `FC-SCHEMA-POST-012` | `redact()` 递归应用 secret；未标记 secret 的字段保持原值；secret 校验失败时不得在 issue 消息/`received` 中泄漏实际值；必填缺失保留 `required` 语义与“缺少必填”说明。 |

## 6. 异常分类

| 类型 / code                     | 触发条件                                          | 关键上下文                              |
| ------------------------------- | ------------------------------------------------- | --------------------------------------- |
| `SchemaValidationError`         | 输入不满足 schema                                 | `issues[]`：code/path/expected/received |
| `SchemaDefinitionError`         | 非法选项、非法默认值、空问题列表、非法 union 分支 | 直接消息、`cause`                       |
| `SchemaError` / `schema_error`  | 通用基类                                          | `code`、`cause`                         |
| `KurumiError` / `invalid_value` | 默认值或内部 JSON 克隆遇到非 JSON 值              | 路径标签                                |

边界采样：

- 字符串：空串、纯空白、Unicode/emoji、超长、`g/y` 正则重复解析。
- 数值：`0`、`-0`、`±1`、边界值、`NaN`、`±Infinity`、`MAX_SAFE_INTEGER`、`1e999`。
- 集合：空数组/空对象、单元素、超过上限、稀疏数组。
- 键：`__proto__`、`prototype`、`constructor`、非法 record 键、未知 object 键。
- 包装器：`optional` + `default`、`nullable` + `default`、嵌套 `secret`。
- 深度：64、65、70 层嵌套。

## 7. 追溯矩阵

| 约束                                     | 测试文件 / 用例                                                                 |
| ---------------------------------------- | ------------------------------------------------------------------------------- |
| `PRE-001` / `PRE-002`                    | `src/scalars.test.ts`、`src/composites.test.ts` · 非法长度/范围与上下界         |
| `PRE-003`                                | `src/scalars.test.ts`、`src/composites.test.ts` · default 校验与深拷贝          |
| `PRE-004`                                | `src/scalars.test.ts` · 空 oneOf/非有限字面量；`src/composites.test.ts` · union |
| `PRE-005`                                | `src/composites.test.ts` · 危险字段名、未知键策略、missingSection               |
| `PRE-006`                                | `src/scalars.test.ts` · 空描述；schema 工具 · 空 parseFailure                   |
| `PRE-007`                                | `src/scalars.test.ts` · 非法布尔选项、g/y 正则稳定性                            |
| `PRE-008` / `POST-002`                   | `src/scalars.test.ts` · 强转开/关与严格数字语法                                 |
| `PRE-009`                                | `src/composites.test.ts` · 非法 shape/子 schema/选项                            |
| `POST-003` passthrough                   | `src/composites.test.ts` · 未知键克隆与非 JSON 值拒绝                           |
| `POST-008` nullable 委托                 | `src/json-schema.test.ts` · `optional().nullable()` 不进入 required             |
| `POST-012` secret 错误脱敏               | `src/scalars.test.ts` · secret 校验错误不泄漏实际值、必填缺失语义               |
| `POST-001`                               | `src/scalars.test.ts` · 标量类型、长度、范围、非有限值                          |
| `POST-003` / `POST-004`                  | `src/composites.test.ts` · object 未知键、missingSection                        |
| `POST-005`                               | `src/composites.test.ts` · array 下标/稀疏数组、record 键规则                   |
| `POST-006` / `POST-007`                  | `src/composites.test.ts` · union 顺序与问题聚合                                 |
| `POST-008`                               | `src/scalars.test.ts`、`src/composites.test.ts` · optional/nullable/default     |
| `POST-009` / `POST-010` / `I-SCHEMA-006` | `src/json-schema.test.ts` · 全量 JSON Schema 映射、冻结与可序列化               |
| `POST-011`                               | `src/scalars.test.ts` · `parseOrThrow` 抛 `SchemaValidationError`               |
| `POST-012` / `I-SCHEMA-008`              | `src/composites.test.ts` · 嵌套 secret、整体 secret、union redact               |
| `I-SCHEMA-001` / `I-SCHEMA-002`          | `src/properties.test.ts` · 解析幂等且不修改输入；object shape 脱钩              |
| `I-SCHEMA-003`                           | `src/composites.test.ts` · 原型污染键                                           |
| `I-SCHEMA-004`                           | `src/composites.test.ts` · 深度防护                                             |
| `I-SCHEMA-005`                           | 全部失败用例断言具体 code/path；无空 catch                                      |
| `I-SCHEMA-007`                           | `src/architecture.test.ts` · 无 node/config/agent/log/console 依赖              |
| `PBT-SCHEMA-001`                         | `src/properties.test.ts` · 任意合法值解析幂等、输入不可变                       |
| `PBT-SCHEMA-002`                         | `src/properties.test.ts` · JSON Schema 导出确定                                 |
| 配置兼容                                 | `packages/config/src/schema/*.test.ts` · 强转、missingSection、错误类型保持不变 |
| 工具兼容                                 | `packages/agent/src/tool.test.ts` · schema 导出、自动 parseInput、类型推导      |

## 8. JSON Schema 映射

| schema                     | JSON Schema 输出                                                                                  |
| -------------------------- | ------------------------------------------------------------------------------------------------- |
| `string()`                 | `{ type: 'string', minLength?, maxLength?, pattern? }`                                            |
| `number()`                 | `{ type: 'number' \| 'integer', minimum?, maximum? }`                                             |
| `boolean()`                | `{ type: 'boolean' }`                                                                             |
| `literal(v)`               | `{ const: v }`                                                                                    |
| `oneOf(values)`            | `{ enum: values }`                                                                                |
| `array(item)`              | `{ type: 'array', items, minItems?, maxItems? }`                                                  |
| `object(shape)`            | `{ type: 'object', properties, required?, additionalProperties }`                                 |
| `record(value)`            | `{ type: 'object', additionalProperties: value, propertyNames?, minProperties?, maxProperties? }` |
| `union([...])`             | `{ anyOf: [...] }`                                                                                |
| `optional()` / `default()` | 沿用内层；对象中不进入 `required`，default 追加 `default`                                         |
| `nullable()`               | `{ anyOf: [inner, { type: 'null' }] }`                                                            |

解析期行为 `shouldTrim`、`isNonEmpty` 的空白语义、字符串强转无法被 JSON Schema 完整表达；契约要求 JSON Schema 仅作为模型侧提示，服务端仍以 `parse()` / `parseOrThrow()` 为准。

## 9. 兼容性与演进

- 破坏性变更：调整公开 schema 名称/选项、问题码、错误码、解析强转默认值、`toJsonSchema()` 输出结构、`Schema` 方法签名。
- 非破坏性：新增 schema 类型、新增可选选项且提供默认值、新增错误码、扩展 JSON Schema 输出关键字。
- `@kurumi/config` 通过适配层保持既有错误类型与默认强转语义；修改适配层时必须同步 `docs/spec/config.md`。
- `@kurumi/agent` 的工具输入 schema 可直接使用本包；修改 `ToolDefinition.inputSchema` 契约时必须同步 `docs/spec/agent.md`。
- 修改公开 API 必须同步更新 TSDoc、README、示例与测试，并在提交信息标注 `!`。
- 契约与代码不一致时以本文件为唯一真实源，先更新契约再改实现。
