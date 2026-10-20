/**
 * @file 确定性 FX 汇率与多跳换算。
 * @description 直接/反向/多跳路径按目标货币 code 升序做 BFS；冲突或缺失汇率显式失败。
 * @module finance/fx
 */

import { Decimal, MAX_DECIMAL_SCALE, type DecimalRounding } from '@kurumi/math';
import { CurrencyMismatchError, FxRateError, MissingFxRateError } from './errors.js';
import {
  createMoney,
  currencyEquals,
  currencyLabel,
  isCurrency,
  isMoney,
  type Currency,
  type Money,
} from './money.js';
import { assertTimestampMs } from './validation.js';

/** 多跳路径中间精度。 */
const FX_RATE_SCALE = 18;

/** 汇率定义：1 base = rate quote。 */
export interface FxRate {
  /** 源货币。 */
  readonly base: Currency;
  /** 目标货币。 */
  readonly quote: Currency;
  /** 汇率，必须 > 0。 */
  readonly rate: Decimal;
  /** 报价时间（epoch 毫秒）。 */
  readonly timestampMs: number;
}

/** 汇率创建输入。 */
export interface CreateFxRateInput {
  /** 源货币。 */
  readonly base: Currency;
  /** 目标货币。 */
  readonly quote: Currency;
  /** 汇率。 */
  readonly rate: Decimal;
  /** 报价时间（epoch 毫秒）。 */
  readonly timestampMs: number;
}

/** 换算选项。 */
export interface FxConversionOptions {
  /** 目标金额小数位；缺省目标货币 decimals。 */
  readonly scale?: number;
  /** 舍入模式，缺省 `half-even`。 */
  readonly rounding?: DecimalRounding;
}

/** FX 转换器。 */
export interface FxConverter {
  /** 构造时提供的汇率快照。 */
  readonly rates: readonly FxRate[];
  /**
   * 查询换算率。
   *
   * @param from - 源货币
   * @param to - 目标货币
   * @returns 1 from = rate to
   */
  rate(from: Currency, to: Currency): Decimal;
  /**
   * 换算金额。
   *
   * @param money - 源金额
   * @param target - 目标货币
   * @param options - 小数位与舍入
   * @returns 新金额
   */
  convert(money: Money, target: Currency, options?: FxConversionOptions): Money;
}

/** 邻接边。 */
interface FxEdge {
  readonly to: Currency;
  readonly rate: Decimal;
}

/**
 * 创建汇率定义。
 *
 * @param input - base/quote/rate/timestamp
 * @returns 冻结汇率
 * @throws {FxRateError} base/quote 相同或 rate 非正
 * @throws {FinanceDefinitionError} 货币或时间非法
 *
 * @example
 * ```ts
 * createFxRate({ base: eur, quote: usd, rate: Decimal.parse('1.08'), timestampMs: 1 });
 * ```
 */
export function createFxRate(input: CreateFxRateInput): FxRate {
  if (!isCurrency(input.base) || !isCurrency(input.quote)) {
    throw new FxRateError('base/quote 必须是合法货币');
  }
  if (input.base.code === input.quote.code) {
    throw new FxRateError(`base 与 quote 不能相同（${input.base.code}）`);
  }
  if (!(input.rate instanceof Decimal) || !input.rate.isPositive()) {
    throw new FxRateError(`rate 必须大于 0（实际值: ${String(input.rate)}）`);
  }
  assertTimestampMs(input.timestampMs, 'fx.timestampMs');
  return Object.freeze({
    base: input.base,
    quote: input.quote,
    rate: input.rate,
    timestampMs: input.timestampMs,
  });
}

/**
 * 反转汇率。
 *
 * @param rate - 原汇率
 * @returns base/quote 交换、rate 取倒数（scale 18、half-even）
 * @throws {FxRateError} 汇率定义非法
 */
export function invertFxRate(rate: FxRate): FxRate {
  if (!isFxRate(rate)) {
    throw new FxRateError('汇率结构非法');
  }
  return Object.freeze({
    base: rate.quote,
    quote: rate.base,
    rate: Decimal.ONE.divide(rate.rate, {
      scale: FX_RATE_SCALE,
      rounding: 'half-even',
    }),
    timestampMs: rate.timestampMs,
  });
}

/**
 * 判断值是否为合法汇率。
 *
 * @param value - 待判断值
 * @returns 结构合法时 true
 */
export function isFxRate(value: unknown): value is FxRate {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  const base = candidate['base'];
  const quote = candidate['quote'];
  const rate = candidate['rate'];
  const timestampMs = candidate['timestampMs'];
  return (
    isCurrency(base) &&
    isCurrency(quote) &&
    base.code !== quote.code &&
    rate instanceof Decimal &&
    rate.isPositive() &&
    typeof timestampMs === 'number' &&
    Number.isSafeInteger(timestampMs) &&
    timestampMs >= 0
  );
}

/**
 * 创建 FX 转换器。
 *
 * @param rates - 汇率列表；每个方向最多一条，且不得同时出现反向冲突
 * @returns 冻结转换器
 * @throws {FxRateError} 重复/反向冲突或汇率非法
 *
 * @example
 * ```ts
 * const fx = createFxConverter([
 *   createFxRate({ base: eur, quote: usd, rate: Decimal.parse('1.08'), timestampMs: 1 }),
 *   createFxRate({ base: usd, quote: jpy, rate: Decimal.parse('150'), timestampMs: 1 }),
 * ]);
 * fx.rate(eur, jpy); // 162
 * ```
 */
export function createFxConverter(rates: readonly FxRate[]): FxConverter {
  const edgeMap = new Map<string, FxEdge[]>();
  const seenDirected = new Set<string>();
  const frozenRates: FxRate[] = [];
  const currenciesByCode = new Map<string, Currency>();

  for (const rate of rates) {
    if (!isFxRate(rate)) {
      throw new FxRateError('createFxConverter 收到非法汇率');
    }
    rememberCurrency(currenciesByCode, rate.base);
    rememberCurrency(currenciesByCode, rate.quote);
    const directedKey = `${rate.base.code}/${rate.quote.code}`;
    const reverseKey = `${rate.quote.code}/${rate.base.code}`;
    if (seenDirected.has(directedKey)) {
      throw new FxRateError(`重复汇率 ${directedKey}`);
    }
    if (seenDirected.has(reverseKey)) {
      throw new FxRateError(`同时提供 ${directedKey} 与 ${reverseKey}，请只提供一侧以避免冲突`);
    }
    seenDirected.add(directedKey);
    frozenRates.push(rate);

    addEdge(edgeMap, rate.base, rate.quote, rate.rate);
    addEdge(
      edgeMap,
      rate.quote,
      rate.base,
      Decimal.ONE.divide(rate.rate, {
        scale: FX_RATE_SCALE,
        rounding: 'half-even',
      }),
    );
  }

  // BFS 邻居按 code 排序，保证同一组汇率得到确定路径。
  for (const edges of edgeMap.values()) {
    edges.sort((left, right) => left.to.code.localeCompare(right.to.code));
  }

  const rateCache = new Map<string, Decimal>();
  const frozenEdgeMap = new Map(
    [...edgeMap.entries()].map(([code, edges]) => [
      code,
      Object.freeze(edges.map((edge) => Object.freeze(edge))),
    ]),
  );

  const converter: FxConverter = {
    rates: Object.freeze([...frozenRates]),
    rate: (from: Currency, to: Currency): Decimal => {
      if (!isCurrency(from) || !isCurrency(to)) {
        throw new FxRateError('from/to 必须是合法货币');
      }
      assertCurrencyDefinition(currenciesByCode, from);
      assertCurrencyDefinition(currenciesByCode, to);
      if (from.code === to.code) {
        if (!currencyEquals(from, to)) {
          throw new CurrencyMismatchError(currencyLabel(from), currencyLabel(to));
        }
        return Decimal.ONE;
      }
      // 缓存必须区分 code + decimals；同 code 不同定义不是同一个货币。
      const cacheKey = `${from.code}:${from.decimals}/${to.code}:${to.decimals}`;
      const cached = rateCache.get(cacheKey);
      if (cached !== undefined) return cached;

      const resolved = findRate(frozenEdgeMap, from, to);
      rateCache.set(cacheKey, resolved);
      return resolved;
    },
    convert: (money: Money, target: Currency, options?: FxConversionOptions): Money => {
      if (!isMoney(money)) {
        throw new FxRateError('convert 的 money 非法');
      }
      if (!isCurrency(target)) {
        throw new FxRateError('convert 的 target 非法');
      }
      if (money.currency.code === target.code) {
        if (!currencyEquals(money.currency, target)) {
          throw new CurrencyMismatchError(currencyLabel(money.currency), currencyLabel(target));
        }
        return createMoney(money.amount, target);
      }

      const scale = options?.scale ?? target.decimals;
      if (!Number.isSafeInteger(scale) || scale < 0 || scale > MAX_DECIMAL_SCALE) {
        throw new FxRateError(`convert scale 必须是 0..${MAX_DECIMAL_SCALE} 的整数`);
      }
      const rate = converter.rate(money.currency, target);
      const amount = money.amount.multiply(rate, {
        scale,
        rounding: options?.rounding ?? 'half-even',
      });
      return createMoney(amount, target);
    },
  };

  return Object.freeze(converter);
}

/**
 * 用单条汇率换算金额。
 *
 * @param money - 源金额
 * @param rate - 汇率；base 必须等于 money.currency
 * @param options - 小数位与舍入
 * @returns 目标金额
 * @throws {CurrencyMismatchError} 金额币种不是 rate.base
 */
export function convertMoneyWithRate(
  money: Money,
  rate: FxRate,
  options?: FxConversionOptions,
): Money {
  if (!isMoney(money)) throw new FxRateError('money 非法');
  if (!isFxRate(rate)) throw new FxRateError('rate 非法');
  if (!currencyEquals(money.currency, rate.base)) {
    throw new CurrencyMismatchError(currencyLabel(money.currency), currencyLabel(rate.base));
  }
  const scale = options?.scale ?? rate.quote.decimals;
  return createMoney(
    money.amount.multiply(rate.rate, {
      scale,
      rounding: options?.rounding ?? 'half-even',
    }),
    rate.quote,
  );
}

/** 记录货币定义；同一 code 的不同 decimals 视为冲突。 */
function rememberCurrency(currenciesByCode: Map<string, Currency>, currency: Currency): void {
  const existing = currenciesByCode.get(currency.code);
  if (existing !== undefined && !currencyEquals(existing, currency)) {
    throw new CurrencyMismatchError(currencyLabel(existing), currencyLabel(currency));
  }
  currenciesByCode.set(currency.code, currency);
}

/** 查询时拒绝与汇率快照不一致的货币定义。 */
function assertCurrencyDefinition(
  currenciesByCode: ReadonlyMap<string, Currency>,
  currency: Currency,
): void {
  const existing = currenciesByCode.get(currency.code);
  if (existing !== undefined && !currencyEquals(existing, currency)) {
    throw new CurrencyMismatchError(currencyLabel(existing), currencyLabel(currency));
  }
}

/** 添加一条邻接边。 */
function addEdge(
  edgeMap: Map<string, FxEdge[]>,
  from: Currency,
  to: Currency,
  rate: Decimal,
): void {
  const key = from.code;
  const edges = edgeMap.get(key);
  const edge = Object.freeze({ to, rate });
  if (edges === undefined) {
    edgeMap.set(key, [edge]);
  } else {
    edges.push(edge);
  }
}

/** BFS 查找换算率。 */
function findRate(
  edges: ReadonlyMap<string, readonly FxEdge[]>,
  from: Currency,
  to: Currency,
): Decimal {
  const visited = new Set<string>([from.code]);
  const queue: Array<{ readonly currency: Currency; readonly rate: Decimal }> = [
    { currency: from, rate: Decimal.ONE },
  ];
  let cursor = 0;

  while (cursor < queue.length) {
    const current = queue[cursor];
    cursor += 1;
    if (current === undefined) continue;
    const neighbors = edges.get(current.currency.code) ?? [];
    for (const edge of neighbors) {
      if (visited.has(edge.to.code)) continue;
      const nextRate = current.rate.multiply(edge.rate, {
        scale: FX_RATE_SCALE,
        rounding: 'half-even',
      });
      if (edge.to.code === to.code) return nextRate;
      visited.add(edge.to.code);
      queue.push({ currency: edge.to, rate: nextRate });
    }
  }

  throw new MissingFxRateError(from.code, to.code);
}
