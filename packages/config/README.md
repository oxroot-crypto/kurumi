# @kurumi/config

Kurumi 配置系统基础设施：**schema 校验 + 多层来源合并 + 环境变量映射 + 字符串插值 + 可重载 store**。

schema 核心由 [`@kurumi/schema`](../schema/README.md) 提供；本包重新导出并保留配置系统默认行为（环境变量字符串强转、对象 section 缺失按空对象解析、配置领域错误类型）。

- **跨 runtime**：核心不依赖 Node/浏览器 API，通过 `RuntimeHost` 注入文件与环境变量能力。
- **零第三方运行时依赖**：仅依赖工作区内的 `@kurumi/core` 与 `@kurumi/schema`。
- **类型安全**：schema 自动推导 readonly 输出类型；非法状态在编译期尽量不可表达。
- **失败显式**：所有错误带稳定 `code`、路径与上下文，聚合多个字段问题。
- **不承担日志**：不调用 `console.*`、不持有 logger；日志级别解析与输出由 `@kurumi/log` 负责。
- **安全默认**：拒绝原型污染键、限制嵌套深度、缺失插值变量报错、结果递归冻结。

## 安装与导入

```ts
import {
  array,
  createConfigStore,
  createMemoryConfigHost,
  envSource,
  fileSource,
  loadConfig,
  number,
  object,
  objectSource,
  oneOf,
  string,
} from '@kurumi/config';
import { createNodeConfigHost } from '@kurumi/config/node';
```

## 快速开始

```ts
import { number, object, string, loadConfig } from '@kurumi/config';
import { createNodeConfigHost } from '@kurumi/config/node';
import { envSource, fileSource } from '@kurumi/config';

const schema = object({
  server: object({
    host: string().default('127.0.0.1'),
    port: number({ isInteger: true, min: 1, max: 65_535 }),
  }),
});

const config = await loadConfig({
  schema,
  host: createNodeConfigHost(),
  sources: [
    fileSource({ path: 'kurumi.jsonc', isOptional: true }),
    envSource({ prefix: 'KURUMI_' }),
  ],
  signal: AbortSignal.timeout(5_000),
});

config.server.port; // number
```

## 来源与优先级

来源按数组顺序从低到高覆盖：

```ts
sources: [
  fileSource({ path: 'kurumi.json' }), // 低
  envSource({ prefix: 'KURUMI_' }),
  objectSource({ name: 'cli', value: overrides }), // 高
];
```

合并规则：

- 普通对象递归合并；数组、标量、`null` 整体替换。
- `undefined` 视为“本层未提供”，不覆盖已有值。
- 根节点必须是普通对象；禁止 `__proto__` / `prototype` / `constructor`。
- 对象 section 缺失默认按 `{}` 处理，使嵌套默认值生效，并让必填叶子节点报告精确路径（如 `$.server.port`）；可用 `object(shape, { missingSection: 'error' })` 改为严格必填。
- 默认值由 schema 在解析阶段填充，不占用来源优先级。

### 文件来源

```ts
fileSource({ path: 'kurumi.jsonc', isOptional: true });
fileSource({ path: '/etc/kurumi/config', format: 'jsonc' });
fileSource({
  path: 'config.any',
  parser: (text, { path }) => parseCustomFormat(text, path),
});
```

- `.json` 使用严格 JSON；`.jsonc` 支持注释、尾随逗号与 BOM。
- 扩展名无法推断时必须显式指定 `format` 或提供 `parser`；运行期 `format` 非法会在构造 `fileSource` 时抛 `ConfigSchemaDefinitionError`。
- `isOptional: true` 只忽略“文件不存在”。
- 语法错误的 `ConfigSyntaxError.sourceName` 是来源名称（如 `file:kurumi.json`），不是绝对路径。

### 环境变量来源

```ts
envSource({ prefix: 'KURUMI_' });
```

| 变量                                 | 结果                                                   |
| ------------------------------------ | ------------------------------------------------------ |
| `KURUMI_SERVER__PORT=3000`           | `{ server: { port: '3000' } }`（schema 强转为 number） |
| `KURUMI_TAGS__0=a` + `__1=b`         | `{ tags: ['a', 'b'] }`                                 |
| `KURUMI_CORS__ORIGINS=["https://a"]` | 直接得到数组                                           |
| `KURUMI_DEBUG=true`                  | `{ debug: 'true' }`（schema 强转为 boolean）           |

- 默认分隔符 `__`；单下划线保留在字段名中。
- 默认小写化路径段；`shouldLowercase: false` 可关闭。
- 数组下标必须连续，且不超过 10000。
- 路径冲突、空路径段、危险键、重复映射都会抛出 `ConfigEnvMappingError`。

## Schema

> schema 定义来自 `@kurumi/schema`；以下 `number` / `boolean` / `literal` / `oneOf` 默认开启字符串强转，`object` 的 `missingSection` 默认 `empty`。直接使用 `@kurumi/schema` 时默认是严格 JSON 语义。

```ts
object(shape, { unknownKeys?: 'error' | 'strip' | 'passthrough', missingSection?: 'empty' | 'error' })
record(valueSchema, { keyPattern?, minKeys?, maxKeys? })
array(itemSchema, { minLength?, maxLength? })
union([schemaA, schemaB]) // 至少两个分支
string({ minLength?, maxLength?, pattern?, shouldTrim?, isNonEmpty? }) // isNonEmpty = shouldTrim 后非空
number({ min?, max?, isInteger?, allowNonFinite?, shouldCoerceStrings? })
boolean({ shouldCoerceStrings? })
literal(value, { shouldCoerceStrings?: boolean })
oneOf(['a', 'b'] as const, { shouldCoerceStrings?: boolean })
```

链式包装器：

```ts
string().optional(); // string | undefined
string().nullable(); // string | null
number().default(3000); // 缺失时使用默认值，构造时立即校验
string().secret(); // redact() 时遮蔽
string().describe('...'); // 文档元数据
```

数值与布尔 schema 默认接受严格字符串（环境变量需要）：

- 数字字符串只接受十进制/科学计数法，拒绝 `0x10`、`1_000`、`12abc`、空串。
- 布尔字符串接受 `true/false/1/0`（大小写不敏感）。
- 可用 `shouldCoerceStrings: false` 关闭；`literal` / `oneOf` 也支持同名选项。
- `number({ allowNonFinite: true })` 是显式逃生口，会允许 `±Infinity`，此类配置不再是严格 JSON，序列化前需自行处理。

## 字符串插值

```jsonc
{
  "database": {
    "url": "${DATABASE_URL:-postgres://localhost:5432/kurumi}",
    "password": "{env:DATABASE_PASSWORD}",
  },
}
```

- 支持 `${NAME}`、`${NAME:-fallback}`、`{env:NAME}` 与 `{env:NAME:-fallback}`。
- 环境变量为空字符串时视为已定义。
- `$${NAME}` 转义为字面量。
- 缺失变量且无默认值默认抛 `ConfigInterpolationError`；可用 `interpolation: { shouldAllowMissing: true }` 保留原文。
- 可用 `interpolation: false` 完全关闭。
- 只处理字符串值，不处理键；环境变量值不会被递归展开。

## 错误处理

```ts
import { ConfigValidationError } from '@kurumi/config';

try {
  await loadConfig({ schema, host, sources });
} catch (error) {
  if (error instanceof ConfigValidationError) {
    const summary = error.issues.map((issue) => `${issue.code} ${issue.path}: ${issue.message}`);
    // summary 交给应用自己的日志/UI 层处理；config 包不直接输出。
  }
}
```

主要错误类型：

| 类型                          | 场景                                       |
| ----------------------------- | ------------------------------------------ |
| `ConfigValidationError`       | schema 校验失败，`issues` 聚合全部字段问题 |
| `ConfigSourceError`           | 来源读取/解析/返回结构失败                 |
| `ConfigSyntaxError`           | JSON/JSONC 语法错误，带 `offset`           |
| `ConfigEnvMappingError`       | 环境变量路径冲突、数组断档等               |
| `ConfigInterpolationError`    | 缺失插值变量或危险键                       |
| `ConfigSchemaDefinitionError` | 非法 schema 定义或非法默认值               |
| `ConfigDisposedError`         | store 已销毁，reload/subscribe 被拒绝      |

所有问题都带 `code`、`path`、`message`，并尽量附 `expected` / `received`。

## 可重载 Store

```ts
const store = await createConfigStore({
  schema,
  host: createNodeConfigHost(),
  sources: [fileSource({ path: 'kurumi.jsonc' }), envSource({ prefix: 'KURUMI_' })],
});

const unsubscribe = store.subscribe(({ config }) => {
  // 应用层在此同步新配置；config 包自身不输出日志。
  applyConfig(config);
});

await store.reload();
unsubscribe();
store.dispose();
```

行为保证：

- 初始加载失败不会返回 store。
- `reload()` 失败时保留上一份可用配置，状态变为 `error`，`lastError` 可读；下一次成功自动恢复。
- 并发 `reload()` 复用同一个 Promise。
- 深比较无变化时不广播；可传 `equals` 自定义；`equals` 抛错会包装为 `compare_error` 并保留旧配置。
- 监听器支持异步；抛错/rejection 默认聚合为 `AggregateError`（此时配置已更新）；可用 `onListenerError` 接管。
- `dispose()` 后 `subscribe` / `reload` 抛出 `ConfigDisposedError`，当前配置仍可读。
- `reload(signal?)` 接受 `AbortSignal`；`loadConfig` 同样支持 `signal`，来源会收到同一个信号。
- `createConfigStore({ signal })` 只作用于首次加载；后续每次 `reload(signal)` 可按次传入。

## 敏感值遮蔽

```ts
const schema = object({
  database: object({
    user: string(),
    password: string().secret(),
  }),
});

const safe = schema.redact(config);
// { database: { user: 'app', password: '[REDACTED]' } }
```

`secret()` 作用于整个子树；联合类型会使用匹配分支的遮蔽规则。

## 跨 runtime 宿主

核心只依赖 `RuntimeHost`：

```ts
interface RuntimeHost {
  readonly cwd: string;
  env(): Readonly<Record<string, string | undefined>>;
  resolvePath(baseDir: string, path: string): string;
  readTextFile(path: string, signal?: AbortSignal): Promise<string>;
}
```

内置实现：

- `createNodeConfigHost()`（`@kurumi/config/node`，实现来自 `@kurumi/core/node`）。
- `createMemoryConfigHost()`（测试、Serverless、浏览器 Worker）。

自定义 runtime 只需实现该接口；若文件不存在，请抛出 `@kurumi/core` 的 `FileNotFoundError`。

## 安全性

- 拒绝 `__proto__` / `prototype` / `constructor`，防止原型污染；插值与 `redact` 同样跳过/拒绝危险键。
- 最大嵌套深度 64（含环境变量结构化 JSON），避免栈溢出与内存放大。
- 环境变量数组下标上限 10000。
- 配置结果递归冻结；类型为 readonly，`get()` 返回的引用不可修改。
- `ConfigSourceKind` 包含 `custom`，自定义 HTTP/Vault/数据库来源无需伪装成内置类型。
- 文件解析不执行代码；`parser` 仅接受调用方显式提供的函数。

## 脚本

```bash
pnpm --filter @kurumi/config typecheck
pnpm --filter @kurumi/config test
pnpm --filter @kurumi/config build
```

契约与追溯矩阵：[`docs/spec/config.md`](../../docs/spec/config.md)
