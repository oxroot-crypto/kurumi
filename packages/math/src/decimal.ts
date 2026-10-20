/**
 * @file 十进制定点数：解析、规范化、四则运算、舍入与步长对齐。
 * @description 使用 `bigint` + 十进制 scale 表示金额/价格/数量，避免 IEEE-754 浮点误差；所有值不可变并自动规范化。
 * @module math/decimal
 */

import {
  DecimalDivisionByZeroError,
  DecimalOverflowError,
  DecimalParseError,
  DecimalRoundingError,
  MathDefinitionError,
} from './errors.js';

/** 最大小数位数；覆盖 crypto 18 位与 FX/利率的额外精度。 */
export const MAX_DECIMAL_SCALE = 30;

/** 除法默认结果 scale。 */
export const DEFAULT_DIVISION_SCALE = 18;

/** 整数部分 + 小数部分的最大位数，防止恶意 BigInt 膨胀。 */
export const MAX_DECIMAL_DIGITS = 1024;

/** 十进制输入字符串最大长度。 */
export const MAX_DECIMAL_INPUT_LENGTH = 2048;

/** 指数绝对值上限，防止 `1e100000` 构造超大整数。 */
const MAX_DECIMAL_EXPONENT = 1000;

/** 系数绝对值上限：10^1024（允许 1024 位，拒绝 1025 位）。 */
const MAX_COEFFICIENT_MAGNITUDE = 10n ** BigInt(MAX_DECIMAL_DIGITS);

/** 舍入模式。 */
export const DecimalRounding = {
  /** 必须精确；出现非零余数时抛错。 */
  Exact: 'exact',
  /** 向零截断。 */
  Down: 'down',
  /** 远离零进位。 */
  Up: 'up',
  /** 向正无穷。 */
  Ceil: 'ceil',
  /** 向负无穷。 */
  Floor: 'floor',
  /** 四舍五入（0.5 远离零）。 */
  HalfUp: 'half-up',
  /** 银行家舍入（0.5 取偶）。 */
  HalfEven: 'half-even',
} as const;

/** 舍入模式联合类型。 */
export type DecimalRounding = (typeof DecimalRounding)[keyof typeof DecimalRounding];

/** 全部合法舍入模式。 */
const ROUNDING_MODES: readonly DecimalRounding[] = Object.freeze(Object.values(DecimalRounding));

/** 乘法选项。 */
export interface DecimalMultiplyOptions {
  /** 目标 scale；缺省保持自然 scale（自然 scale 超过上限时抛错）。 */
  readonly scale?: number;
  /** 目标 scale 下的舍入模式；缺省 `exact`。 */
  readonly rounding?: DecimalRounding;
}

/** 除法选项。 */
export interface DecimalDivisionOptions {
  /** 目标 scale；缺省 18。 */
  readonly scale?: number;
  /** 舍入模式；缺省 `half-even`。 */
  readonly rounding?: DecimalRounding;
}

/**
 * 不可变十进制定点数。
 *
 * @remarks
 * - 值 = `units / 10^scale`。
 * - 零始终表示为 `units = 0n, scale = 0`。
 * - 非零值移除尾随零，因此 `1.50` 与 `1.5` 表示相同。
 * - 所有运算返回新实例；实例在构造时冻结。
 */
export class Decimal {
  /** 缩放后的整数系数。 */
  readonly units: bigint;

  /** 小数位数，范围 `0..MAX_DECIMAL_SCALE`。 */
  readonly scale: number;

  /** 零。 */
  static readonly ZERO: Decimal = new Decimal(0n, 0);

  /** 一。 */
  static readonly ONE: Decimal = new Decimal(1n, 0);

  private constructor(units: bigint, scale: number) {
    this.units = units;
    this.scale = scale;
    Object.freeze(this);
  }

  /**
   * 从整数系数与 scale 构造。
   *
   * @param units - 缩放后的整数系数
   * @param scale - 小数位数，`0..30`
   * @returns 规范化且冻结的 Decimal
   * @throws {DecimalOverflowError} scale 越界、units 不是 bigint 或数值位数超限
   *
   * @example
   * ```ts
   * Decimal.fromUnits(12345n, 2).toString(); // "123.45"
   * ```
   */
  static fromUnits(units: bigint, scale: number): Decimal {
    if (typeof units !== 'bigint') {
      throw new DecimalOverflowError(`Decimal.units 必须是 bigint（实际值: ${String(units)}）`);
    }
    assertScale(scale, 'Decimal.fromUnits');

    if (units === 0n) return Decimal.ZERO;

    let normalizedUnits = units;
    let normalizedScale = scale;
    while (normalizedScale > 0 && normalizedUnits % 10n === 0n) {
      normalizedUnits /= 10n;
      normalizedScale -= 1;
    }

    const absolute = normalizedUnits < 0n ? -normalizedUnits : normalizedUnits;
    if (absolute >= MAX_COEFFICIENT_MAGNITUDE) {
      throw new DecimalOverflowError(
        `Decimal 数值位数超过上限 ${MAX_DECIMAL_DIGITS} 位（scale=${normalizedScale}）`,
      );
    }

    return new Decimal(normalizedUnits, normalizedScale);
  }

  /**
   * 解析十进制字符串。
   *
   * @param text - 可选符号 + 十进制数字 + 可选指数；不接受空白、`NaN`、`Infinity`、十六进制与下划线
   * @returns 规范化且冻结的 Decimal
   * @throws {DecimalParseError} 语法非法
   * @throws {DecimalOverflowError} scale 或位数超限
   *
   * @example
   * ```ts
   * Decimal.parse('-1.25e2').toString(); // "-125"
   * Decimal.parse('.5').toString(); // "0.5"
   * ```
   */
  static parse(text: string): Decimal {
    if (typeof text !== 'string') {
      throw new DecimalParseError(String(text), '输入必须是字符串');
    }
    if (text.length === 0 || text.length > MAX_DECIMAL_INPUT_LENGTH) {
      throw new DecimalParseError(
        text,
        `长度必须在 1..${MAX_DECIMAL_INPUT_LENGTH} 之间（实际 ${text.length}）`,
      );
    }

    let body = text;
    let isNegative = false;
    const first = body.charAt(0);
    if (first === '+' || first === '-') {
      isNegative = first === '-';
      body = body.slice(1);
    }
    if (body.length === 0) {
      throw new DecimalParseError(text, '缺少数字部分');
    }

    const exponentIndex = findExponentIndex(body);
    const mantissa = exponentIndex === -1 ? body : body.slice(0, exponentIndex);
    const exponentText = exponentIndex === -1 ? '0' : body.slice(exponentIndex + 1);
    if (!isValidMantissa(mantissa) || !isValidExponent(exponentText)) {
      throw new DecimalParseError(
        text,
        '语法必须为 [+-]?(digits[.digits]|.digits)([eE][+-]?digits)?',
      );
    }

    const exponent = exponentText === '0' ? 0 : Number(exponentText);
    if (!Number.isSafeInteger(exponent) || Math.abs(exponent) > MAX_DECIMAL_EXPONENT) {
      throw new DecimalParseError(text, `指数绝对值不能超过 ${MAX_DECIMAL_EXPONENT}`);
    }

    const dotIndex = mantissa.indexOf('.');
    const integerText = dotIndex === -1 ? mantissa : mantissa.slice(0, dotIndex);
    const rawFractionText = dotIndex === -1 ? '' : mantissa.slice(dotIndex + 1);
    const fractionText = trimTrailingZeros(rawFractionText);
    const digits = `${integerText}${fractionText}`;
    if (digits === '') {
      return Decimal.ZERO;
    }

    let units = BigInt(digits);
    let scale = fractionText.length - exponent;

    if (units === 0n) return Decimal.ZERO;
    if (isNegative) units = -units;

    if (scale < 0) {
      units *= pow10(-scale);
      scale = 0;
    }
    if (scale > MAX_DECIMAL_SCALE) {
      throw new DecimalOverflowError(
        `Decimal 解析结果 scale=${scale} 超过上限 ${MAX_DECIMAL_SCALE}（输入: ${text}）`,
      );
    }

    return Decimal.fromUnits(units, scale);
  }

  /**
   * 从安全整数构造。
   *
   * @param value - `bigint` 或安全整数 `number`
   * @returns scale 0 的 Decimal
   * @throws {DecimalOverflowError} number 非安全整数
   */
  static fromInteger(value: bigint | number): Decimal {
    if (typeof value === 'bigint') return Decimal.fromUnits(value, 0);
    if (!Number.isSafeInteger(value)) {
      throw new DecimalOverflowError(
        `Decimal.fromInteger 只接受安全整数（实际值: ${String(value)}）`,
      );
    }
    return Decimal.fromUnits(BigInt(value), 0);
  }

  /**
   * 求和。
   *
   * @param values - Decimal 列表；空列表返回零
   * @returns 精确和
   */
  static sum(values: readonly Decimal[]): Decimal {
    let total = Decimal.ZERO;
    for (const value of values) total = total.add(value);
    return total;
  }

  /**
   * 加法。
   *
   * @param other - 另一个 Decimal
   * @returns 精确和
   * @throws {MathDefinitionError} other 不是 Decimal
   */
  add(other: Decimal): Decimal {
    assertOperand(other, 'add');
    if (this.scale === other.scale) {
      return Decimal.fromUnits(this.units + other.units, this.scale);
    }
    const targetScale = Math.max(this.scale, other.scale);
    return Decimal.fromUnits(
      alignUnits(this.units, this.scale, targetScale) +
        alignUnits(other.units, other.scale, targetScale),
      targetScale,
    );
  }

  /**
   * 减法。
   *
   * @param other - 被减数
   * @returns 精确差
   * @throws {MathDefinitionError} other 不是 Decimal
   */
  subtract(other: Decimal): Decimal {
    assertOperand(other, 'subtract');
    return this.add(other.negate());
  }

  /**
   * 乘法。
   *
   * @param other - 乘数
   * @param options - 可选目标 scale 与舍入模式
   * @returns 乘积；未指定 scale 且自然 scale <= 30 时精确
   * @throws {DecimalOverflowError} 自然 scale 超过上限
   * @throws {DecimalRoundingError} 指定 `exact` 但目标 scale 不足
   */
  multiply(other: Decimal, options?: DecimalMultiplyOptions): Decimal {
    assertOperand(other, 'multiply');
    const naturalUnits = this.units * other.units;
    const naturalScale = this.scale + other.scale;

    if (options?.scale !== undefined) {
      const targetScale = options.scale;
      assertScale(targetScale, 'Decimal.multiply');
      return Decimal.fromUnits(
        rescaleUnits(
          naturalUnits,
          naturalScale,
          targetScale,
          options.rounding ?? DecimalRounding.Exact,
          'Decimal.multiply',
        ),
        targetScale,
      );
    }

    if (naturalScale > MAX_DECIMAL_SCALE) {
      throw new DecimalOverflowError(
        `乘法自然 scale=${naturalScale} 超过上限 ${MAX_DECIMAL_SCALE}，请显式指定 scale 与舍入模式`,
      );
    }
    return Decimal.fromUnits(naturalUnits, naturalScale);
  }

  /**
   * 除法。
   *
   * @param other - 除数，不能为零
   * @param options - 目标 scale（缺省 18）与舍入模式（缺省 half-even）
   * @returns 商
   * @throws {DecimalDivisionByZeroError} 除数为零
   * @throws {DecimalOverflowError} scale 越界
   * @throws {DecimalRoundingError} 指定 `exact` 但无法精确表示
   *
   * @example
   * ```ts
   * Decimal.parse('1').divide(Decimal.parse('3')); // 0.333333333333333333
   * ```
   */
  divide(other: Decimal, options?: DecimalDivisionOptions): Decimal {
    assertOperand(other, 'divide');
    if (other.units === 0n) {
      throw new DecimalDivisionByZeroError('Decimal.divide');
    }

    const targetScale = options?.scale ?? DEFAULT_DIVISION_SCALE;
    assertScale(targetScale, 'Decimal.divide');
    const rounding = options?.rounding ?? DecimalRounding.HalfEven;

    let numerator = this.units;
    let denominator = other.units;
    const exponent = other.scale + targetScale - this.scale;
    if (exponent >= 0) {
      numerator *= pow10(exponent);
    } else {
      denominator *= pow10(-exponent);
    }

    return Decimal.fromUnits(
      divideAndRound(numerator, denominator, rounding, 'Decimal.divide', targetScale),
      targetScale,
    );
  }

  /**
   * 按目标 scale 重新缩放。
   *
   * @param scale - 目标 scale
   * @param rounding - 舍入模式，缺省 `exact`
   * @returns 重新缩放后的 Decimal
   * @throws {DecimalRoundingError} 目标 scale 不足且无法精确表示
   */
  rescale(scale: number, rounding: DecimalRounding = DecimalRounding.Exact): Decimal {
    assertScale(scale, 'Decimal.rescale');
    assertRounding(rounding, 'Decimal.rescale');
    if (scale === this.scale) return this;
    return Decimal.fromUnits(
      rescaleUnits(this.units, this.scale, scale, rounding, 'Decimal.rescale'),
      scale,
    );
  }

  /**
   * 舍入到指定 scale。
   *
   * @param scale - 目标 scale
   * @param rounding - 舍入模式，缺省 `half-even`
   * @returns 舍入后的 Decimal
   */
  round(scale: number, rounding: DecimalRounding = DecimalRounding.HalfEven): Decimal {
    return this.rescale(scale, rounding);
  }

  /**
   * 比较数值。
   *
   * @param other - 另一个 Decimal
   * @returns 负数/零/正数分别表示小于/等于/大于
   */
  compare(other: Decimal): -1 | 0 | 1 {
    assertOperand(other, 'compare');
    if (this.scale === other.scale) {
      return compareBigInt(this.units, other.units);
    }
    const targetScale = Math.max(this.scale, other.scale);
    return compareBigInt(
      alignUnits(this.units, this.scale, targetScale),
      alignUnits(other.units, other.scale, targetScale),
    );
  }

  /**
   * 判断数值相等。
   *
   * @param other - 另一个 Decimal
   * @returns 数值相等时 true
   */
  equals(other: Decimal): boolean {
    return this.compare(other) === 0;
  }

  /**
   * 符号。
   *
   * @returns `-1` / `0` / `1`
   */
  sign(): -1 | 0 | 1 {
    if (this.units < 0n) return -1;
    if (this.units > 0n) return 1;
    return 0;
  }

  /** @returns 是否为零 */
  isZero(): boolean {
    return this.units === 0n;
  }

  /** @returns 是否为负 */
  isNegative(): boolean {
    return this.units < 0n;
  }

  /** @returns 是否为正 */
  isPositive(): boolean {
    return this.units > 0n;
  }

  /** @returns 是否为整数（规范表示下 scale 为 0） */
  isInteger(): boolean {
    return this.scale === 0;
  }

  /**
   * 绝对值。
   *
   * @returns 非负 Decimal
   */
  abs(): Decimal {
    return this.units < 0n ? this.negate() : this;
  }

  /**
   * 取反。
   *
   * @returns 相反数；零取反仍为零
   */
  negate(): Decimal {
    return this.units === 0n ? Decimal.ZERO : Decimal.fromUnits(-this.units, this.scale);
  }

  /**
   * 规范十进制字符串。
   *
   * @returns 无指数、无尾随零的文本
   */
  toString(): string {
    return formatUnits(this.units, this.scale);
  }

  /**
   * 固定小数位格式化。
   *
   * @param scale - 小数位数，`0..30`；缺省使用当前 scale
   * @param rounding - 舍入模式，缺省 `half-even`
   * @returns 恰好 `scale` 位小数的文本
   * @throws {DecimalOverflowError} scale 越界
   */
  toFixed(
    scale: number = this.scale,
    rounding: DecimalRounding = DecimalRounding.HalfEven,
  ): string {
    assertScale(scale, 'Decimal.toFixed');
    return formatUnits(
      rescaleUnits(this.units, this.scale, scale, rounding, 'Decimal.toFixed'),
      scale,
    );
  }

  /**
   * 有损转换为 number。
   *
   * @remarks 仅用于与需要 `number` 的数值库/模型交互；账务计算不得使用。
   * @returns IEEE-754 近似值
   * @throws {DecimalOverflowError} 结果溢出为 `±Infinity`
   */
  toNumber(): number {
    const value = Number(this.toString());
    if (!Number.isFinite(value)) {
      throw new DecimalOverflowError(`Decimal 转换 number 溢出（值: ${this.toString()}）`);
    }
    return value;
  }

  /**
   * JSON 序列化为规范十进制字符串。
   *
   * @returns 与 `toString()` 相同
   */
  toJSON(): string {
    return this.toString();
  }
}

/**
 * 判断值是否为 Decimal。
 *
 * @param value - 待判断值
 * @returns 是 Decimal 实例时 true
 */
export function isDecimal(value: unknown): value is Decimal {
  return value instanceof Decimal;
}

/**
 * 求和（函数形式）。
 *
 * @param values - Decimal 列表
 * @returns 精确和
 */
export function sumDecimals(values: readonly Decimal[]): Decimal {
  return Decimal.sum(values);
}

/**
 * 取最小值。
 *
 * @param values - 非空 Decimal 列表
 * @returns 最小元素
 * @throws {MathDefinitionError} 列表为空
 */
export function minDecimal(values: readonly Decimal[]): Decimal {
  const first = values[0];
  if (first === undefined) {
    throw new MathDefinitionError('minDecimal 至少需要一个值');
  }
  let minimum = first;
  for (const value of values) {
    if (value.compare(minimum) < 0) minimum = value;
  }
  return minimum;
}

/**
 * 取最大值。
 *
 * @param values - 非空 Decimal 列表
 * @returns 最大元素
 * @throws {MathDefinitionError} 列表为空
 */
export function maxDecimal(values: readonly Decimal[]): Decimal {
  const first = values[0];
  if (first === undefined) {
    throw new MathDefinitionError('maxDecimal 至少需要一个值');
  }
  let maximum = first;
  for (const value of values) {
    if (value.compare(maximum) > 0) maximum = value;
  }
  return maximum;
}

/**
 * 判断是否为步长的整数倍。
 *
 * @param value - 待判断值
 * @param step - 正步长（例如 tickSize）
 * @returns 是整数倍时 true
 * @throws {MathDefinitionError} step 非正或参数不是 Decimal
 */
export function isDecimalMultipleOf(value: Decimal, step: Decimal): boolean {
  assertOperand(value, 'isDecimalMultipleOf');
  assertOperand(step, 'isDecimalMultipleOf');
  if (!step.isPositive()) {
    throw new MathDefinitionError(`步长必须大于 0（实际值: ${step.toString()}）`);
  }
  const commonScale = Math.max(value.scale, step.scale);
  const valueUnits = alignUnits(value.units, value.scale, commonScale);
  const stepUnits = alignUnits(step.units, step.scale, commonScale);
  return valueUnits % stepUnits === 0n;
}

/**
 * 将值对齐到步长的整数倍。
 *
 * @param value - 待对齐值
 * @param step - 正步长
 * @param rounding - 舍入模式，缺省 `half-even`
 * @returns 步长整数倍
 * @throws {MathDefinitionError} step 非正或参数不是 Decimal
 */
export function roundDecimalToStep(
  value: Decimal,
  step: Decimal,
  rounding: DecimalRounding = DecimalRounding.HalfEven,
): Decimal {
  assertOperand(value, 'roundDecimalToStep');
  assertOperand(step, 'roundDecimalToStep');
  if (!step.isPositive()) {
    throw new MathDefinitionError(`步长必须大于 0（实际值: ${step.toString()}）`);
  }

  const commonScale = Math.max(value.scale, step.scale);
  const valueUnits = alignUnits(value.units, value.scale, commonScale);
  const stepUnits = alignUnits(step.units, step.scale, commonScale);
  const quotient = divideAndRound(valueUnits, stepUnits, rounding, 'roundDecimalToStep', 0);
  return Decimal.fromUnits(quotient * stepUnits, commonScale);
}

/** 把 units 按指定 scale 格式化为固定小数位文本。 */
function formatUnits(units: bigint, scale: number): string {
  const isNegative = units < 0n;
  const absolute = isNegative ? -units : units;
  const digits = absolute.toString().padStart(scale + 1, '0');
  if (scale === 0) return `${isNegative ? '-' : ''}${digits}`;
  const integerPart = digits.slice(0, digits.length - scale);
  const fractionPart = digits.slice(digits.length - scale);
  return `${isNegative ? '-' : ''}${integerPart}.${fractionPart}`;
}

/** 查找指数标记位置；无则 -1。 */
function findExponentIndex(body: string): number {
  const lower = body.indexOf('e');
  const upper = body.indexOf('E');
  if (lower === -1) return upper;
  if (upper === -1) return lower;
  return Math.min(lower, upper);
}

/** 校验尾数语法。 */
function isValidMantissa(mantissa: string): boolean {
  return /^(?:\d+(?:\.\d+)?|\.\d+)$/.test(mantissa);
}

/** 校验指数语法。 */
function isValidExponent(exponent: string): boolean {
  return /^[+-]?\d+$/.test(exponent);
}

/** 移除尾随零。 */
function trimTrailingZeros(value: string): string {
  let end = value.length;
  while (end > 0 && value.charAt(end - 1) === '0') end -= 1;
  return value.slice(0, end);
}

/** 校验 scale。 */
function assertScale(scale: number, operation: string): void {
  if (!Number.isSafeInteger(scale) || scale < 0 || scale > MAX_DECIMAL_SCALE) {
    throw new DecimalOverflowError(
      `${operation} 的 scale 必须是 0..${MAX_DECIMAL_SCALE} 的安全整数（实际值: ${String(scale)}）`,
    );
  }
}

/** 校验舍入模式。 */
function assertRounding(rounding: DecimalRounding, operation: string): void {
  if (!ROUNDING_MODES.includes(rounding)) {
    throw new MathDefinitionError(`${operation} 的舍入模式非法（实际值: ${String(rounding)}）`);
  }
}

/** 校验操作数。 */
function assertOperand(value: Decimal, operation: string): void {
  if (!(value instanceof Decimal)) {
    throw new MathDefinitionError(
      `Decimal.${operation} 的操作数必须是 Decimal（实际值: ${String(value)}）`,
    );
  }
}

/** 计算 10^n；仅接受非负安全整数。 */
function pow10(exponent: number): bigint {
  if (!Number.isSafeInteger(exponent) || exponent < 0) {
    throw new DecimalOverflowError(`10 的指数必须是非负安全整数（实际值: ${String(exponent)}）`);
  }
  return 10n ** BigInt(exponent);
}

/** 把 units 从 fromScale 对齐到 toScale（toScale >= fromScale）。 */
function alignUnits(units: bigint, fromScale: number, toScale: number): bigint {
  return toScale === fromScale ? units : units * pow10(toScale - fromScale);
}

/** 比较两个 bigint。 */
function compareBigInt(left: bigint, right: bigint): -1 | 0 | 1 {
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

/**
 * 把 units 从 fromScale 缩放到 toScale，并按模式舍入。
 *
 * @param units - 原始系数
 * @param fromScale - 原始 scale
 * @param toScale - 目标 scale
 * @param rounding - 舍入模式
 * @param operation - 操作名
 * @returns 目标 scale 下的整数系数
 */
function rescaleUnits(
  units: bigint,
  fromScale: number,
  toScale: number,
  rounding: DecimalRounding,
  operation: string,
): bigint {
  assertRounding(rounding, operation);
  if (toScale >= fromScale) {
    return toScale === fromScale ? units : units * pow10(toScale - fromScale);
  }
  return divideAndRound(units, pow10(fromScale - toScale), rounding, operation, toScale);
}

/**
 * 对有理数 numerator/denominator 做整数除法并按模式舍入。
 *
 * @param numerator - 分子
 * @param denominator - 分母，不能为 0
 * @param rounding - 舍入模式
 * @param operation - 操作名
 * @param scale - 目标 scale，仅用于错误上下文
 * @returns 舍入后的整数
 */
function divideAndRound(
  numerator: bigint,
  denominator: bigint,
  rounding: DecimalRounding,
  operation: string,
  scale: number,
): bigint {
  if (denominator === 0n) throw new DecimalDivisionByZeroError(operation);
  assertRounding(rounding, operation);

  const isNegative = numerator < 0n !== denominator < 0n;
  const absoluteNumerator = numerator < 0n ? -numerator : numerator;
  const absoluteDenominator = denominator < 0n ? -denominator : denominator;
  const quotient = absoluteNumerator / absoluteDenominator;
  const remainder = absoluteNumerator % absoluteDenominator;

  if (remainder === 0n) return isNegative ? -quotient : quotient;

  let rounded = quotient;
  switch (rounding) {
    case DecimalRounding.Exact:
      throw new DecimalRoundingError(operation, scale);
    case DecimalRounding.Down:
      break;
    case DecimalRounding.Up:
      rounded += 1n;
      break;
    case DecimalRounding.Ceil:
      if (!isNegative) rounded += 1n;
      break;
    case DecimalRounding.Floor:
      if (isNegative) rounded += 1n;
      break;
    case DecimalRounding.HalfUp:
      if (remainder * 2n >= absoluteDenominator) rounded += 1n;
      break;
    case DecimalRounding.HalfEven: {
      const doubled = remainder * 2n;
      if (
        doubled > absoluteDenominator ||
        (doubled === absoluteDenominator && quotient % 2n === 1n)
      ) {
        rounded += 1n;
      }
      break;
    }
    default: {
      const exhaustive: never = rounding;
      throw new DecimalOverflowError(`未知舍入模式: ${String(exhaustive)}`);
    }
  }

  return isNegative ? -rounded : rounded;
}
