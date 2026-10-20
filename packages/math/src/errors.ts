/**
 * @file 数学包错误体系。
 * @description 复用 core 的 `KurumiError`，区分数学定义错误、运行时值域错误与小数/期权专用错误。
 * @module math/errors
 */

import { KurumiError, type KurumiErrorOptions } from '@kurumi/core';

/** 数学包所有错误的基类。 */
export class MathError extends KurumiError {
  /**
   * @param message - 面向人类的错误描述
   * @param options - 错误码与 cause
   */
  constructor(message: string, options?: KurumiErrorOptions) {
    super(message, { code: 'math_error', ...options });
  }
}

/** 数学定义/工厂输入非法。 */
export class MathDefinitionError extends MathError {
  /**
   * @param message - 非法的定义或选项描述
   * @param cause - 底层定义期错误
   */
  constructor(message: string, cause?: unknown) {
    super(message, {
      code: 'math_definition_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}

/** 运行时数值/参数/序列非法。 */
export class MathValueError extends MathError {
  /**
   * @param message - 非法值描述
   * @param cause - 底层错误
   */
  constructor(message: string, cause?: unknown) {
    super(message, {
      code: 'math_value_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}

/** 十进制字符串语法非法。 */
export class DecimalParseError extends MathError {
  /** 被拒绝的原始输入。 */
  readonly input: string;

  /**
   * @param input - 被拒绝的原始输入
   * @param reason - 语法失败原因
   */
  constructor(input: string, reason: string) {
    const preview = input.length > 80 ? `${input.slice(0, 80)}…` : input;
    super(`无法解析十进制数 "${preview}": ${reason}`, { code: 'decimal_parse_error' });
    this.input = input;
  }
}

/** 精确舍入遇到非零余数。 */
export class DecimalRoundingError extends MathError {
  /** 触发舍入的操作名。 */
  readonly operation: string;
  /** 目标 scale。 */
  readonly scale: number;

  /**
   * @param operation - 操作名
   * @param scale - 目标 scale
   */
  constructor(operation: string, scale: number) {
    super(`${operation} 在 scale=${scale} 下无法精确表示，请显式指定舍入模式`, {
      code: 'decimal_rounding_error',
    });
    this.operation = operation;
    this.scale = scale;
  }
}

/** 除数为零。 */
export class DecimalDivisionByZeroError extends MathError {
  /** 触发除零的操作名。 */
  readonly operation: string;

  /**
   * @param operation - 操作名
   */
  constructor(operation: string) {
    super(`${operation}: 除数不能为零`, { code: 'decimal_division_by_zero' });
    this.operation = operation;
  }
}

/** 小数 scale 或数值位数超出上限。 */
export class DecimalOverflowError extends MathError {
  /**
   * @param message - 越界描述
   */
  constructor(message: string) {
    super(message, { code: 'decimal_overflow' });
  }
}

/** 期权定价参数非法、价格越界或隐含波动率未收敛。 */
export class OptionModelError extends MathError {
  /**
   * @param message - 模型失败描述
   */
  constructor(message: string) {
    super(`期权模型失败: ${message}`, { code: 'option_model_error' });
  }
}
