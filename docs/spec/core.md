# @kurumi/core 形式化契约

> 版本：0.1.0\
> 范围：`packages/core/src` 的错误基类、通用边界校验、JSON 值与错误序列化、RuntimeHost 抽象与内存/Node 宿主。\
> 判定：本包是跨包基础设施，属于「对外契约类模块」；涉及正确性、安全与资源边界的推荐项按强制处理。

## 1. 适用范围与豁免判定

`@kurumi/core` 不承载业务状态机，但承载所有上层包共享的错误语义、值边界与 runtime 抽象，因此**不适用** FSVDD §1.2 的纯文档/构建配置豁免。持久化契约采用本文件；作为对外契约类模块，已引入 `fast-check` 属性测试覆盖 JSON 克隆/比较/序列化不变量，变异测试仍作为后续增量项。

## 2. 类型与不变量

### 2.1 类型

```text
JsonPrimitive = string | number | boolean | null
JsonValue     = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue }
RuntimeHost   = { cwd, env(), resolvePath(), readTextFile(path, signal?) }
KurumiError   = Error & { code: string, cause?: unknown }
ErrorLike     = { name: string, message: string, stack?: string, cause?: unknown }
SerializedError = { name, message, stack?, cause?, errors?, [key: string]: JsonValue | undefined }
```

### 2.2 系统不变量

| 编号         | 不变量                                                                                                                                                         |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `I-CORE-001` | `cloneJsonValue` 的输出只包含 JSON 风格值（数值必须有限），且与输入不共享对象/数组引用。                                                                       |
| `I-CORE-002` | 任何接受用户数据的入口都不得因 `__proto__` / `prototype` / `constructor` 污染 `Object.prototype`。                                                             |
| `I-CORE-003` | 嵌套深度超过 `MAX_JSON_DEPTH = 64` 时必须显式失败，不得栈溢出。                                                                                                |
| `I-CORE-004` | 核心入口（`@kurumi/core`）不得导入 `node:` 内置模块；只有 `@kurumi/core/node` 可以。                                                                           |
| `I-CORE-005` | 所有错误必须带稳定 `code`；跨层错误必须保留 `cause`。                                                                                                          |
| `I-CORE-006` | `deepFreeze` 之后，对象与数组的任意嵌套成员均不可写。                                                                                                          |
| `I-CORE-007` | `toJsonSafeValue` / `serializeError` 的输出只包含 JSON 安全值；循环引用、超深嵌套、危险键与非有限数值必须被显式标记或丢弃，不得抛栈或污染 `Object.prototype`。 |

## 3. 形式化约束矩阵

### 3.1 前置约束

| 编号              | 约束                                                                                                                | 实现                        | 失败语义                                                                |
| ----------------- | ------------------------------------------------------------------------------------------------------------------- | --------------------------- | ----------------------------------------------------------------------- |
| `FC-CORE-PRE-001` | `cloneJsonValue` 输入为 JSON 风格值；拒绝函数、`Date`、稀疏数组、`BigInt`、`Symbol`、符号键与 `NaN`/`±Infinity`。   | `cloneJsonValue`            | 抛 `KurumiError('invalid_value')`                                       |
| `FC-CORE-PRE-002` | 对象键不得为原型污染键。                                                                                            | `isForbiddenKey`            | 抛 `KurumiError('forbidden_key')`                                       |
| `FC-CORE-PRE-003` | 递归深度 `d <= 64`，且内部 `depth` 必须是非负安全整数。                                                             | `cloneJsonValue`            | 抛 `KurumiError('max_depth_exceeded')` / `KurumiError('invalid_depth')` |
| `FC-CORE-PRE-004` | `RuntimeHost.readTextFile` 的文件不存在必须抛 `FileNotFoundError`；取消必须保留 `AbortError`，不得包装为读取失败。  | `node-host` / `memory-host` | 上层可据此实现 optional 文件与取消传播                                  |
| `FC-CORE-PRE-005` | 路径以 POSIX 语义规范化；`..` 不得越过根。                                                                          | `normalizePosixPath`        | 截断到 `/`                                                              |
| `FC-CORE-PRE-006` | `toJsonSafeValue` / `serializeError` 的 `maxDepth >= 0`、`maxStringLength >= 1`、`maxEntries >= 1` 且均为安全整数。 | `serialize.ts`              | 抛 `KurumiError('invalid_options')`                                     |

### 3.2 后置约束

| 编号               | 约束                                                                                                                                                                          | 实现                   |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| `FC-CORE-POST-001` | 克隆结果深结构相等且引用独立。                                                                                                                                                | `cloneJsonValue`       |
| `FC-CORE-POST-002` | `deepFreeze` 返回原引用并将全部嵌套对象/数组冻结。                                                                                                                            | `deepFreeze`           |
| `FC-CORE-POST-003` | `isDeepEqual` 忽略对象键顺序；`NaN` 等于 `NaN`；`-0` 不等于 `0`；非普通对象不视为相等；循环引用不会无限递归；使用迭代遍历避免深层输入栈溢出。                                 | `isDeepEqual`          |
| `FC-CORE-POST-004` | 子类错误 `name` 等于子类名，`code` 为显式值或缺省值。                                                                                                                         | `KurumiError` 构造函数 |
| `FC-CORE-POST-005` | 内存/Node 宿主解析相对路径时以 `baseDir` 为基准；绝对路径保持绝对。                                                                                                           | `resolvePath`          |
| `FC-CORE-POST-006` | `toJsonSafeValue` 对循环引用输出 `[Circular]`，超过深度输出 `[MaxDepth]`；超过条目/字符串上限时截断并标记，包括 Symbol/BigInt/函数名与对象键；Map/Set/Date 等均有确定性转换。 | `serialize.ts`         |
| `FC-CORE-POST-007` | `serializeError` 默认保留 `stack`、`cause` 与自身可枚举字段；`AggregateError` 的 `errors[]` 逐项序列化；返回结构递归冻结。                                                    | `serialize.ts`         |

### 3.3 异常与边界

| 编号               | 场景                      | 期望                                            |
| ------------------ | ------------------------- | ----------------------------------------------- |
| `FC-CORE-ERR-001`  | 稀疏数组                  | `invalid_value`                                 |
| `FC-CORE-ERR-002`  | 危险键                    | `forbidden_key`                                 |
| `FC-CORE-ERR-003`  | 超深嵌套                  | `max_depth_exceeded`                            |
| `FC-CORE-ERR-003b` | 内部 depth 为负数/小数    | `invalid_depth`                                 |
| `FC-CORE-ERR-004`  | 文件不存在                | `FileNotFoundError`，`code = file_not_found`    |
| `FC-CORE-ERR-005`  | 非 ENOENT 读取失败        | `RuntimeHostError`，`code = runtime_host_error` |
| `FC-CORE-ERR-006`  | 空/纯空白路径段的规范化   | 直接丢弃，不产生空段                            |
| `FC-CORE-ERR-007`  | 序列化循环引用 / 超深嵌套 | 输出 `[Circular]` / `[MaxDepth]`，不抛栈溢出    |
| `FC-CORE-ERR-008`  | 序列化选项越界            | `KurumiError('invalid_options')`                |

### 3.4 通用边界校验

| 编号                | 约束                                                                                                                                           | 失败语义                     |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------- |
| `FC-CORE-VALID-001` | 提供字符串、非空白字符串、正则、非负/正安全整数、时间戳、有限数、布尔、封闭字符串集合、普通对象、数组长度与闭区间校验；闭区间拒绝 `NaN`/无穷。 | 缺省抛 `CoreValidationError` |
| `FC-CORE-VALID-002` | 每个校验函数接受可选 `ValidationErrorFactory`；领域包可据此映射为 `FinanceDefinitionError`、`MathDefinitionError` 等领域错误。                 | —                            |
| `FC-CORE-VALID-003` | `describeValue` 对任意值输出最多 80 字符描述；不得因 `String()` 抛错或泄漏超长输入。                                                           | —                            |

## 4. 追溯矩阵

| 约束                                                                | 测试文件 / 用例                                                                                                                                     |
| ------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FC-CORE-PRE-001` / `POST-001`                                      | `src/value.test.ts` · `cloneJsonValue` · 深拷贝/拒绝函数、Date、稀疏数组、符号键                                                                    |
| `FC-CORE-PRE-002`                                                   | `src/value.test.ts` · 拒绝危险键并保留路径                                                                                                          |
| `FC-CORE-PRE-003` / `ERR-003b`                                      | `src/value.test.ts` · 超过深度上限 / 接受等于深度上限 / 非法内部 depth                                                                              |
| `FC-CORE-PRE-001` 非有限数值                                        | `src/value.test.ts` · `NaN` / `±Infinity` 被拒绝                                                                                                    |
| `FC-CORE-POST-003` 深层迭代                                         | `src/value.test.ts` · 20k 层深比较不栈溢出                                                                                                          |
| `FC-CORE-POST-002`                                                  | `src/value.test.ts` · `deepFreeze`                                                                                                                  |
| `FC-CORE-POST-003`                                                  | `src/value.test.ts` · `isDeepEqual`（含循环引用）                                                                                                   |
| `FC-CORE-POST-004` / `ERR-*`                                        | `src/errors.test.ts`                                                                                                                                |
| `FC-CORE-PRE-004` / `POST-005` / `ERR-004`                          | `src/memory-host.test.ts` · `src/node/node-host.test.ts`（含取消保留 `AbortError`）                                                                 |
| `FC-CORE-ERR-005`                                                   | `src/node/node-host.test.ts` · 其他读取错误包装为 RuntimeHostError                                                                                  |
| `FC-CORE-PRE-006` / `POST-006` / `POST-007` / `ERR-007` / `ERR-008` | `src/serialize.test.ts` · 选项校验、循环/深度、特殊值、错误链与 AggregateError                                                                      |
| `I-CORE-004`                                                        | `src/architecture.test.ts` · 核心代码不直接依赖 node:；`packages/config/src/architecture.test.ts`、`packages/log/src/architecture.test.ts` 交叉校验 |
| `I-CORE-007`                                                        | `src/serialize.test.ts` · 危险键、循环引用与 JSON 安全输出                                                                                          |
| `PBT-CORE-001`                                                      | `src/properties.test.ts` · 任意 JSON 值的克隆/自反/可序列化属性                                                                                     |
| `FC-CORE-VALID-*`                                                   | `src/validation.test.ts` · 字符串/数值/集合/对象校验、自定义错误工厂与描述截断                                                                      |

## 5. 兼容性与演进

- `RuntimeHost` 新增必选成员属于破坏性变更，必须升 major 并提供适配器；`readTextFile(path, signal?)` 的 signal 为可选参数，属于非破坏性扩展。
- `SerializeValueOptions.includeStack` 已重命名为 `shouldIncludeStack`，属于破坏性变更；调用方需同步更新选项名。
- `RuntimeHostError` 固定 `code = runtime_host_error`，不再接受调用方覆盖；移除 code 选项视为破坏性变更。
- 错误码、`MAX_JSON_DEPTH`、危险键集合、序列化占位符（`[Circular]` / `[MaxDepth]` / `[truncated]`）是对外契约；调整视为破坏性变更。
- 新增 `toJsonSafeValue` / `serializeError` 属于非破坏性扩展；新增可选序列化选项必须提供缺省值。
- 新增 `isAbortSignal` / `isAbortError` 属于非破坏性扩展，供各包统一取消判定。
- 新增通用 `validation` 导出属于非破坏性扩展；各包可注入领域错误工厂，缺省行为抛 `CoreValidationError`。
- 新增可选宿主方法必须提供缺省行为，不得要求现有实现同步修改。
