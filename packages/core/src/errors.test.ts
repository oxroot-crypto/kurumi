/**
 * @file core/errors 单元测试。
 * @description 固定 code、cause 与子类 name 契约。
 * @module core/errors.test
 */

import { describe, expect, it } from 'vitest';

import { FileNotFoundError, KurumiError, RuntimeHostError } from './errors.js';

describe('KurumiError', () => {
  it('缺省 code 为 kurumi_error', () => {
    const error = new KurumiError('boom');
    expect(error.code).toBe('kurumi_error');
    expect(error.name).toBe('KurumiError');
    expect(error.message).toBe('boom');
  });

  it('保留自定义 code 与 cause', () => {
    const cause = new Error('root');
    const error = new KurumiError('boom', { code: 'custom', cause });
    expect(error.code).toBe('custom');
    expect(error.cause).toBe(cause);
  });
});

describe('FileNotFoundError', () => {
  it('携带路径并使用子类 name', () => {
    const error = new FileNotFoundError('/tmp/missing.json');
    expect(error.name).toBe('FileNotFoundError');
    expect(error.code).toBe('file_not_found');
    expect(error.path).toBe('/tmp/missing.json');
    expect(error.message).toContain('/tmp/missing.json');
  });
});

describe('RuntimeHostError', () => {
  it('固定 code 并可覆盖 cause', () => {
    const cause = new Error('EACCES');
    const error = new RuntimeHostError('读取失败', { cause });
    expect(error.name).toBe('RuntimeHostError');
    expect(error.code).toBe('runtime_host_error');
    expect(error.cause).toBe(cause);
  });

  it('运行期传入 code 也不能覆盖固定错误码', () => {
    const error = new RuntimeHostError('读取失败', { code: 'custom' } as never);
    expect(error.code).toBe('runtime_host_error');
  });
});
