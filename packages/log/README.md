# @kurumi/log

Kurumi 的跨 runtime 日志基础设施：**级别过滤 + 结构化字段 + 可插拔 sink + 错误链序列化**。

- **跨 runtime**：主入口不依赖 `node:`；Node 文件 sink 只出现在 `@kurumi/log/node`。
- **结构化**：所有字段先归一化为 JSON 安全值，循环引用、Error、BigInt、Map/Set 都有稳定表示。
- **不吞错**：同步 sink 失败直接抛出；异步失败交给 `onSinkError`，否则由 `flush()`/`close()` 显式聚合抛出。
- **无全局状态**：日志器由应用显式创建和注入；config 包不读取、不持有日志器。
- **不可变记录**：交给 sink 的记录已递归冻结，多个 sink 看到同一份数据。

## 安装与导入

```ts
import {
  LogLevel,
  createConsoleSink,
  createLogger,
  createMemorySink,
  parseLogLevel,
} from '@kurumi/log';
import { createNodeFileSink } from '@kurumi/log/node';
```

## 快速开始

```ts
import { LogLevel, createLogger } from '@kurumi/log';

const logger = createLogger({ level: LogLevel.Info });
const requestLogger = logger.child({ service: 'api', requestId: 'req-1' });

requestLogger.info('server started', { port: 3000 });
requestLogger.error('request failed', { error: new Error('upstream timeout') });

await logger.close();
```

默认输出到 console，默认级别为 `info`。

## 级别

```ts
export const LogLevel = {
  Trace: 'trace',
  Debug: 'debug',
  Info: 'info',
  Warn: 'warn',
  Error: 'error',
  Fatal: 'fatal',
  Silent: 'silent',
} as const;
```

- 阈值过滤：只有严重程度大于等于阈值才写入；`silent` 关闭全部输出。
- `silent` 只能作为阈值，不能写入。
- `parseLogLevel()` 用于把配置/环境变量字符串转换为级别，大小写不敏感：

```ts
const level = parseLogLevel(process.env.LOG_LEVEL ?? '') ?? LogLevel.Info;
const logger = createLogger({ level });
```

`@kurumi/config` 只负责产出字符串，不内置日志级别或日志器。

## 结构化字段

```ts
const logger = createLogger({ bindings: { service: 'worker' } });

logger.info('job done', {
  jobId: 'j-1',
  durationMs: 120,
  error: new Error('root cause'),
});

logger.warn('large payload', { payload }); // 自动截断/标记循环引用
```

字段规则：

- `Error` → `{ name, message, stack?, code?, cause?, ... }`，默认保留调用栈。
- 循环引用 → `[Circular]`；超过深度 → `[MaxDepth]`。
- `undefined` 字段被忽略；危险键（`__proto__` / `prototype` / `constructor`）被丢弃。
- 顶层用户字段数量遵守 `maxEntries`（超过时额外增加一个 `[truncated]` 元数据键）；`message` 遵守 `maxMessageLength`（缺省等于 `maxStringLength`，截断标记计入长度上限）。
- 字段 provider 抛错时降级为 `[fields_error]` 字段，不会反向中断业务。
- 延迟字段可传函数，仅在级别启用后求值：

```ts
logger.debug('dump', () => ({ snapshot: expensiveSnapshot() }));
```

## 子 logger 与级别覆盖

```ts
const root = createLogger({ bindings: { service: 'api' } });
const dbLogger = root.child({ component: 'db' });
const verboseLogger = dbLogger.withLevel(LogLevel.Debug);
```

- `child()` 浅合并 bindings，子级覆盖父级，不修改父级。
- `withLevel()` 只替换阈值，保留 bindings、sink 与生命周期。
- 同一组 logger 共享 sink；任意一个 `close()` 都会关闭整组 sink。

## 内置 sink

| sink      | 导入                                         | 说明                                  |
| --------- | -------------------------------------------- | ------------------------------------- |
| console   | `createConsoleSink()`                        | 同步输出 pretty 文本或单行 JSON       |
| memory    | `createMemorySink()`                         | 测试/诊断；`records()`、`clear()`     |
| Node 文件 | `createNodeFileSink()`（`@kurumi/log/node`） | JSONL 或 pretty，自动建目录、串行写入 |

```ts
import { createConsoleSink, createLogger } from '@kurumi/log';
import { createNodeFileSink } from '@kurumi/log/node';

const logger = createLogger({
  sinks: [createConsoleSink({ format: 'pretty' }), createNodeFileSink({ path: 'logs/app.jsonl' })],
});
```

自定义 sink 只需实现 `write(record)`，可选 `flush()` / `close()`：

```ts
import type { LogSink } from '@kurumi/log';

const sink: LogSink = {
  name: 'http',
  async write(record) {
    await fetch('https://logs.example.com/ingest', {
      method: 'POST',
      body: JSON.stringify(record),
    });
  },
};
```

## 失败处理与关闭

```ts
const logger = createLogger({
  sinks: [sink],
  onSinkError: ({ error, sink, record }) => {
    // 可降级到本地文件/内存，不应抛出
    report(error, sink.name, record.level);
  },
});

logger.info('x');
await logger.flush(); // 等待在飞行写入，刷新 sink；未处理失败会在这里抛出 AggregateError
await logger.close(); // flush + 依次关闭 sink，幂等
```

- 同步 sink 抛错时，其余 sink 仍会被尝试；无 handler 时抛 `LogSinkError`（多个失败为 `AggregateError`）。
- 异步 sink reject 时立即交给 `onSinkError`；没有 handler 时由 `flush()`/`close()` 抛出。
- `onSinkError` 支持异步；其 rejection 会进入错误队列，由 `flush()`/`close()` 暴露。
- `flush()` 会等待到没有在飞行写入为止，包括 flush 开始后新增的写入。
- `close()` 先 flush，再按声明顺序关闭 sink；并发 `close()` 复用同一个 Promise，不会提前返回。
- `flush()`/`close()` 自身的 sink 刷新或关闭失败总是以 `AggregateError` 抛出。
- `close()` 后继续写入抛 `LogClosedError`；重复 `close()` 不会重复释放句柄。
- 直接使用 Node 文件 sink 时，未观察的 `write()` 失败会在后续 `flush()`/`close()` 中显式抛出。

## 与 config 配合

```ts
import { loadConfig } from '@kurumi/config';
import { createLogger, parseLogLevel } from '@kurumi/log';

const config = await loadConfig({ schema, host, sources });
const level = parseLogLevel(config.logLevel) ?? 'info';
const logger = createLogger({ level });
```

这样配置包只关心字符串 schema，级别语义与输出行为完全由日志包负责。

## 脚本

```bash
pnpm --filter @kurumi/log typecheck
pnpm --filter @kurumi/log test
pnpm --filter @kurumi/log build
```

契约：[`docs/spec/log.md`](../../docs/spec/log.md)
