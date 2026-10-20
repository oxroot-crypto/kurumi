/**
 * @file 配置领域错误测试。
 * @description 覆盖稳定 code、cause、子类 name 与空 issues 的定义期拒绝。
 * @module config/errors.test
 */

import { describe, expect, it } from 'vitest';

import {
  ConfigEnvMappingError,
  ConfigError,
  ConfigInterpolationError,
  ConfigSchemaDefinitionError,
  ConfigSourceError,
  ConfigValidationError,
} from './errors.js';
import { ConfigIssueCode, createIssue } from './issue.js';

const ISSUE = createIssue(ConfigIssueCode.Required, '$.port', '缺少必填字段');

describe('配置错误', () => {
  it('保留 code、name 与 cause', () => {
    const cause = new Error('root');
    const error = new ConfigSourceError('file', '读取失败', { cause });
    expect(error.name).toBe('ConfigSourceError');
    expect(error.code).toBe('source_error');
    expect(error.cause).toBe(cause);
    expect(error.message).toContain('file');
  });

  it('校验错误聚合 issues', () => {
    const error = new ConfigValidationError([ISSUE]);
    expect(error.code).toBe('validation_error');
    expect(error.issues).toEqual([ISSUE]);
    expect(Object.isFrozen(error.issues)).toBe(true);
  });

  it('拒绝空 issues 列表', () => {
    expect(() => new ConfigValidationError([])).toThrow(ConfigSchemaDefinitionError);
    expect(() => new ConfigInterpolationError([])).toThrow(ConfigSchemaDefinitionError);
    expect(() => new ConfigEnvMappingError('env', [])).toThrow(ConfigSchemaDefinitionError);
  });

  it('基础 ConfigError 允许自定义 code', () => {
    expect(new ConfigError('x', { code: 'custom' }).code).toBe('custom');
    expect(new ConfigError('x').code).toBe('config_error');
  });
});
