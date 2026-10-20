/**
 * @file 金融工具领域错误。
 * @description 工具执行期的标的/货币/参数/能力错误；由 Agent 归一化为模型可见结果。
 * @module finance-tools/errors
 */

import { KurumiError } from '@kurumi/core';

/** 金融 Agent 工具错误。 */
export class FinanceToolError extends KurumiError {
  /**
   * @param message - 失败描述
   * @param cause - 底层错误
   */
  constructor(message: string, cause?: unknown) {
    super(`金融工具失败: ${message}`, {
      code: 'finance_tool_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}
