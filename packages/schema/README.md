# @kurumi/schema

Kurumi 的通用 schema 基础设施：**类型安全解析 + 结构化错误 + JSON Schema 导出**。

- **统一入口**：配置（`@kurumi/config`）与工具输入（`@kurumi/agent`）共用同一套 schema 定义。
- **严格默认**：默认按 JSON 语义校验，不隐式把 `"3000"` 转成 `3000`；需要环境变量强转时显式开启。
- **跨 runtime**：只依赖 `@kurumi/core`，不导入 `node:`、不写日志。
- **零第三方运行时依赖**：不引入 zod/valibot，保持包体积与可审计性。
- **失败显式**：定义期抛 `SchemaDefinitionError`，校验期抛 `SchemaValidationError`，二者都带稳定 `code`。
- **安全默认**：拒绝原型污染键、限制嵌套深度、默认对象被克隆、结果可递归冻结。

## 安装与导入

```ts
import {
  array,
  boolean,
  literal,
  number,
  object,
  oneOf,
  record,
  string,
  union,
} from '@kurumi/schema';
```

## 快速开始

```ts
import { array, number, object, string } from '@kurumi/schema';

const serverSchema = object({
  host: string({ isNonEmpty: true }).default('127.0.0.1'),
  port: number({ isInteger: true, min: 1, max: 65_535 }),
  tags: array(string(), { maxLength: 8 }).default([]),
});

const config = serverSchema.parseOrThrow({ port: 3000 });
// { readonly host: '127.0.0.1', readonly port: 3000, readonly tags: [] }
```

## Schema API

```ts
object(shape, { unknownKeys?: 'error' | 'strip' | 'passthrough', missingSection?: 'empty' | 'error' })
record(valueSchema, { keyPattern?, minKeys?, maxKeys? })
array(itemSchema, { minLength?, maxLength? })
union([schemaA, schemaB]) // 至少两个分支
string({ minLength?, maxLength?, pattern?, shouldTrim?, isNonEmpty? })
number({ min?, max?, isInteger?, allowNonFinite?, shouldCoerceStrings? })
boolean({ shouldCoerceStrings? })
literal(value, { shouldCoerceStrings?: boolean })
oneOf(['a', 'b'] as const, { shouldCoerceStrings?: boolean })
```

链式包装器：

```ts
string().optional(); // string | undefined
string().nullable(); // string | null
number().default(3000); // 缺失时使用默认值，构造时立即校验并克隆
string().secret(); // redact() 时替换为 [REDACTED]
string().describe('监听地址'); // 元数据，同时写入 JSON Schema description
```

## JSON Schema 导出

`Schema.toJsonSchema()` 输出冻结、JSON 安全的 2020-12 风格子集，可直接用于工具定义：

```ts
import { number, object, string } from '@kurumi/schema';
import { createTool } from '@kurumi/agent';

const tool = createTool({
  name: 'search',
  description: '搜索文档',
  inputSchema: object({
    query: string({ isNonEmpty: true }).describe('搜索词'),
    limit: number({ isInteger: true, min: 1, max: 20 }).default(5),
  }),
  async execute({ query, limit }, { signal }) {
    return await searchDocs(query, limit, { signal });
  },
});
```

- `optional()` / `default()` 字段不会进入 `required`。
- `unknownKeys: 'strip'` 与 `passthrough` 导出 `additionalProperties: true`，因为解析器都会接受额外输入（前者随后丢弃）。
- `shouldTrim`、字符串强转等**解析期行为无法用 JSON Schema 表达**，模型侧描述与实际服务端校验可能略有差异。
- `secret()` 只影响 `redact()`，不会写入 JSON Schema。

## 错误处理

```ts
import { SchemaDefinitionError, SchemaValidationError } from '@kurumi/schema';

try {
  schema.parseOrThrow(input);
} catch (error) {
  if (error instanceof SchemaValidationError) {
    for (const issue of error.issues) {
      console.log(issue.code, issue.path, issue.message);
    }
  }
  throw error;
}
```

| 类型                    | code                      | 场景                                 |
| ----------------------- | ------------------------- | ------------------------------------ |
| `SchemaValidationError` | `schema_validation_error` | 输入不满足 schema，`issues` 聚合问题 |
| `SchemaDefinitionError` | `schema_definition_error` | 非法选项、非法默认值、空问题列表     |
| `SchemaError`           | `schema_error`            | 通用基类                             |

问题码：`required`、`invalid_type`、`unknown_key`、`forbidden_key`、`too_small`、`too_large`、`pattern_mismatch`、`not_integer`、`not_finite`、`invalid_literal`、`union_mismatch`、`invalid_value`、`max_depth_exceeded`。

## 与 @kurumi/config 的关系

`@kurumi/config` 从本包重新导出 schema，并保留配置系统需要的默认行为：

- `number()` / `boolean()` / `literal()` / `oneOf()` 默认开启环境变量字符串强转；
- `object()` 的 `missingSection` 默认 `empty`，让嵌套默认值生效；
- 解析错误转换为 `ConfigValidationError`，定义错误转换为 `ConfigSchemaDefinitionError`。

直接使用 `@kurumi/schema` 时保持严格 JSON 默认值，适合工具输入等边界。

## 安全

- 拒绝 `__proto__` / `prototype` / `constructor`。
- 最大嵌套深度 64，超限返回 `max_depth_exceeded`。
- 默认值在构造时校验并深拷贝，调用方修改原对象不会污染 schema。
- 解析不修改输入；结果中的对象由 schema 重新构造。
- `toJsonSchema()` 结果递归冻结。

## 脚本

```bash
pnpm --filter @kurumi/schema typecheck
pnpm --filter @kurumi/schema test
pnpm --filter @kurumi/schema test:coverage
pnpm --filter @kurumi/schema build
```

契约与追溯矩阵：[`docs/spec/schema.md`](../../docs/spec/schema.md)
