/**
 * @file Node 文件 sink 单元测试。
 * @description 覆盖目录创建、JSONL/pretty 输出、追加/截断、关闭与非法选项。
 * @module log/node/file-sink.test
 */

import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { LogDefinitionError, LogSinkError } from '../errors.js';
import type { LogFormat } from '../format.js';
import { createLogger } from '../logger.js';
import { createNodeFileSink, type NodeFileSinkOptions } from './file-sink.js';

let directory = '';

beforeEach(async () => {
  directory = await mkdtemp(join(tmpdir(), 'kurumi-log-'));
});

afterEach(async () => {
  await rm(directory, { recursive: true, force: true });
});

describe('createNodeFileSink', () => {
  it('自动创建父目录并写入 JSONL', async () => {
    const path = join(directory, 'nested', 'app.jsonl');
    const sink = createNodeFileSink({ path });
    const logger = createLogger({
      sinks: [sink],
      clock: () => 0,
      bindings: { service: 'api' },
    });

    logger.info('one', { count: 1 });
    logger.warn('two');
    await logger.close();

    expect(sink.path).toBe(path);
    const lines = (await readFile(path, 'utf8')).trim().split('\n');
    expect(lines).toHaveLength(2);
    expect(JSON.parse(lines[0] ?? '')).toEqual({
      timeMs: 0,
      level: 'info',
      message: 'one',
      bindings: { service: 'api' },
      fields: { count: 1 },
    });
    expect(JSON.parse(lines[1] ?? '')).toMatchObject({ level: 'warn', message: 'two' });
  });

  it('shouldAppend=true 追加，shouldAppend=false 截断', async () => {
    const path = join(directory, 'app.jsonl');

    const first = createLogger({ sinks: [createNodeFileSink({ path })] });
    first.info('first');
    await first.close();

    const second = createLogger({ sinks: [createNodeFileSink({ path })] });
    second.info('second');
    await second.close();

    expect((await readFile(path, 'utf8')).trim().split('\n')).toHaveLength(2);

    const truncated = createLogger({
      sinks: [createNodeFileSink({ path, shouldAppend: false })],
    });
    truncated.info('third');
    await truncated.close();

    const lines = (await readFile(path, 'utf8')).trim().split('\n');
    expect(lines).toHaveLength(1);
    expect(JSON.parse(lines[0] ?? '')).toMatchObject({ message: 'third' });
  });

  it('pretty 格式输出可读文本', async () => {
    const path = join(directory, 'app.log');
    const logger = createLogger({
      sinks: [createNodeFileSink({ path, format: 'pretty' })],
      clock: () => 0,
    });
    logger.error('failed', { code: 'E' });
    await logger.close();

    const text = await readFile(path, 'utf8');
    expect(text).toBe('1970-01-01T00:00:00.000Z ERROR failed fields={"code":"E"}\n');
  });

  it('关闭后直接写入被拒绝', async () => {
    const sink = createNodeFileSink({ path: join(directory, 'app.jsonl') });
    await sink.close();
    const error = await catchAsync(() =>
      sink.write({
        timeMs: 0,
        level: 'info',
        message: 'x',
        bindings: {},
      }),
    );
    expect(error).toBeInstanceOf(LogSinkError);
  });

  it('直接使用时 flush 会暴露未观察的写入错误', async () => {
    const blocked = join(directory, 'blocked');
    await writeFile(blocked, 'not a directory');
    const sink = createNodeFileSink({ path: join(blocked, 'app.jsonl') });
    try {
      await sink.write({ timeMs: 0, level: 'info', message: 'x', bindings: {} });
    } catch {
      // 预期写入失败；随后 flush 必须显式暴露同一错误。
    }
    await expect(sink.flush()).rejects.toBeInstanceOf(LogSinkError);
  });

  it('直接 write 失败也包装为 LogSinkError，成功重试后不会残留旧错误', async () => {
    const blocked = join(directory, 'blocked-retry');
    await writeFile(blocked, 'not a directory');
    const sink = createNodeFileSink({ path: join(blocked, 'app.jsonl') });
    const record = { timeMs: 0, level: 'info' as const, message: 'x', bindings: {} };

    const firstError = await catchAsync(() => sink.write(record));
    expect(firstError).toBeInstanceOf(LogSinkError);

    await rm(blocked);
    await mkdir(blocked, { recursive: true });
    await expect(sink.write(record)).resolves.toBeUndefined();
    await expect(sink.flush()).resolves.toBeUndefined();
    await sink.close();
  });

  it('关闭后 flush 安全完成', async () => {
    const sink = createNodeFileSink({ path: join(directory, 'app.jsonl') });
    await sink.close();
    await expect(sink.flush()).resolves.toBeUndefined();
  });

  it('拒绝非法选项', () => {
    const invalidNullOptions: NodeFileSinkOptions = JSON.parse('null');
    expect(() => createNodeFileSink(invalidNullOptions)).toThrow(LogDefinitionError);
    expect(() => createNodeFileSink({ path: ' ' })).toThrow(LogDefinitionError);
    const invalidFormat: string = 'xml';
    expect(() =>
      createNodeFileSink({ path: join(directory, 'a'), format: invalidFormat as LogFormat }),
    ).toThrow(LogDefinitionError);
    const invalidOptions: NodeFileSinkOptions = JSON.parse(
      JSON.stringify({ path: join(directory, 'a'), shouldAppend: 'yes' }),
    );
    expect(() => createNodeFileSink(invalidOptions)).toThrow(LogDefinitionError);

    const invalidDirectory: NodeFileSinkOptions = JSON.parse(
      JSON.stringify({ path: join(directory, 'a'), shouldEnsureDirectory: 'yes' }),
    );
    expect(() => createNodeFileSink(invalidDirectory)).toThrow(LogDefinitionError);
  });
});

async function catchAsync(run: () => Promise<unknown>): Promise<unknown> {
  try {
    return await run();
  } catch (error) {
    return error;
  }
}
