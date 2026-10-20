# @kurumi/core

Kurumi 的跨 runtime 基础设施包。它只承载与业务无关、可被多个包复用的能力：

- 统一错误基类 `KurumiError`（稳定 `code`、`cause` 因果链、正确子类名）。
- JSON 风格值工具（判定、深拷贝、冻结、深比较、原型污染防护）。
- 任意值/错误对象的 JSON 安全序列化（循环引用、深度、截断、错误链）。
- `RuntimeHost` 抽象：文件读取、环境变量与路径解析。
- 两种宿主实现：Node（`@kurumi/core/node`）与内存（测试/浏览器 Worker/Serverless）。

> 核心入口不导入任何 `node:` 内置模块；Node 能力只出现在 `@kurumi/core/node`。

## 快速开始

```ts
import { cloneJsonValue, deepFreeze, isDeepEqual, serializeError } from '@kurumi/core';
import { createNodeRuntimeHost } from '@kurumi/core/node';

const host = createNodeRuntimeHost({ cwd: process.cwd() });
const text = await host.readTextFile(host.resolvePath(host.cwd, 'kurumi.jsonc'));

const value = cloneJsonValue(JSON.parse(text), '$.root');
deepFreeze(value);
isDeepEqual(value, value); // true

serializeError(new Error('boom', { cause: new Error('root') }));
// { name: 'Error', message: 'boom', stack: '...', cause: { ... } }
```

## RuntimeHost

```ts
interface RuntimeHost {
  readonly cwd: string;
  env(): Readonly<Record<string, string | undefined>>;
  resolvePath(baseDir: string, path: string): string;
  readTextFile(path: string, signal?: AbortSignal): Promise<string>;
}
```

约定：

- `env()` 在每次调用时返回当前环境；Node 宿主显式传入的 `env` 会在创建时冻结为快照，未传入时读取实时 `process.env`。
- `readTextFile` 接受可选 `AbortSignal`；文件不存在时必须抛出 `FileNotFoundError`，其他读取失败应包装为 `RuntimeHostError`（code 固定为 `runtime_host_error`）。
- 上层可以基于该接口实现浏览器 `fetch` 宿主、Deno/Bun 宿主或测试内存宿主。

## 取消判定

`isAbortSignal(value)` 与 `isAbortError(value)` 提供跨 realm 的结构化判断：前者识别 `AbortSignal` 形状，后者识别 `AbortError` / `ABORT_ERR`，供 Host、config 与 agent 复用。

## JSON 值工具

| API              | 说明                                               |
| ---------------- | -------------------------------------------------- |
| `isPlainObject`  | 只接受对象字面量与 `Object.create(null)`           |
| `isForbiddenKey` | 识别 `__proto__` / `prototype` / `constructor`     |
| `cloneJsonValue` | 深拷贝并拒绝函数、Date、稀疏数组、危险键、超深嵌套 |
| `deepFreeze`     | 递归冻结对象/数组                                  |
| `isDeepEqual`    | 结构比较，忽略键顺序，遵循 `Object.is` 语义        |
| `MAX_JSON_DEPTH` | 默认嵌套上限 64                                    |

## JSON 安全序列化

| API                                                                                         | 说明                                                                                                        |
| ------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `toJsonSafeValue`                                                                           | 将任意值转换为 JSON 安全结构；循环引用 `[Circular]`、超深 `[MaxDepth]`、超限截断标记                        |
| `serializeError`                                                                            | 序列化 Error 的 `name` / `message` / `stack` / `code` / `cause` / `AggregateError.errors`，返回递归冻结结构 |
| `isErrorLike`                                                                               | 识别具备字符串 `name` 与 `message` 的错误形态                                                               |
| `DEFAULT_SERIALIZE_DEPTH` / `DEFAULT_SERIALIZE_STRING_LENGTH` / `DEFAULT_SERIALIZE_ENTRIES` | 序列化默认上限（8 / 8192 / 1000）                                                                           |

所有转换都不会修改输入，并会丢弃 `__proto__` / `prototype` / `constructor` 等危险键。`@kurumi/log` 的结构化字段与错误字段直接复用该模块。

## 脚本

```bash
pnpm --filter @kurumi/core typecheck
pnpm --filter @kurumi/core test
pnpm --filter @kurumi/core build
```

契约：[`docs/spec/core.md`](../../docs/spec/core.md)
