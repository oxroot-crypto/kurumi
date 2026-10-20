/**
 * @file 配置问题路径与展示工具测试。
 * @description 覆盖根路径、数组/对象类型描述与 issue 文本格式。
 * @module config/issue.test
 */

import { describe, expect, it } from 'vitest';

import {
  ConfigIssueCode,
  createIssue,
  describeType,
  displayPath,
  formatIssues,
  ROOT_PATH,
} from './issue.js';

describe('displayPath', () => {
  it('根路径显示为 <root>，其他路径保持原样', () => {
    expect(displayPath(ROOT_PATH)).toBe('<root>');
    expect(displayPath('$.server.port')).toBe('$.server.port');
  });
});

describe('describeType', () => {
  it('区分 null、数组与 typeof 结果', () => {
    expect(describeType(null)).toBe('null');
    expect(describeType([1])).toBe('array');
    expect(describeType('x')).toBe('string');
    expect(describeType(1)).toBe('number');
  });
});

describe('formatIssues', () => {
  it('按路径与消息渲染多行文本', () => {
    const issues = [
      createIssue(ConfigIssueCode.Required, '$.server.port', '缺少字段'),
      createIssue(ConfigIssueCode.InvalidType, '$.debug', '期望 boolean'),
    ];
    expect(formatIssues(issues)).toBe('  - $.server.port: 缺少字段\n  - $.debug: 期望 boolean');
  });
});
