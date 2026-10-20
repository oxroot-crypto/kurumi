/**
 * @file 金融分析类 Agent 工具。
 * @description 包装 finance/math 的纯计算能力；不访问网络、不依赖 provider。
 * @module finance-tools/analysis-tools
 */

import { createTool, type ToolDefinition } from '@kurumi/agent';
import {
  OrderSide,
  adjustHistoricPrices,
  applyFillToPosition,
  bondCleanPrice,
  bondDirtyPrice,
  bondYield,
  checkOrderRisk,
  convexity,
  createBondSpec,
  createCashDividend,
  createCurrency,
  createFill,
  createMoney,
  createPosition,
  createStockDividend,
  createStockSplit,
  couponPerPeriod,
  accruedInterest,
  divideMoney,
  dv01,
  formatMoney,
  macaulayDuration,
  modifiedDuration,
  multiplyMoney,
  addMoney,
  subtractMoney,
  compareMoney,
  parseMoney,
  positionMarkToMarket,
  type CorporateAction,
  type RiskLimits,
} from '@kurumi/finance';
import {
  Decimal,
  OptionKind,
  annualizedReturn,
  annualizedVolatility,
  blackScholes,
  expectedShortfall,
  historicalVaR,
  impliedVolatility,
  maxDrawdown,
  sharpeRatio,
  simpleReturns,
} from '@kurumi/math';
import { array, number, object, oneOf } from '@kurumi/schema';

import { FinanceToolError } from './errors.js';
import { toolResult } from './json.js';
import { createToolContext, type FinanceToolOptions } from './options.js';
import { parseDateUtc } from './provider-tools.js';
import {
  CURRENCY_CODE_SCHEMA,
  DATE_SCHEMA,
  DECIMAL_STRING_SCHEMA,
  FILL_INPUT_SCHEMA,
  INSTRUMENT_KEY_SCHEMA,
  MONEY_SCHEMA,
  OPTION_TYPE_SCHEMA,
  POSITIVE_INTEGER_SCHEMA,
  PRICE_POINT_SCHEMA,
  SIDE_SCHEMA,
  TIMESTAMP_MS_SCHEMA,
} from './schemas.js';

/** 金额运算。 */
const MONEY_OPERATION_SCHEMA = oneOf([
  'add',
  'subtract',
  'multiply',
  'divide',
  'compare',
  'format',
] as const).describe('金额运算：add/subtract/multiply/divide/compare/format');

/** 债券日计数。 */
const DAY_COUNT_SCHEMA = oneOf([
  'actual/actual',
  'actual/360',
  'actual/365',
  '30/360',
] as const).describe('债券日计数规则，例如 actual/actual、30/360');

/** 公司行为。 */
const CORPORATE_ACTION_SCHEMA = object({
  type: oneOf(['split', 'stock_dividend', 'cash_dividend'] as const).describe(
    '公司行为类型：split 拆股、stock_dividend 股票股利、cash_dividend 现金分红',
  ),
  exDateMs: TIMESTAMP_MS_SCHEMA.describe('除权/除息日（epoch 毫秒）'),
  ratio: DECIMAL_STRING_SCHEMA.describe(
    '拆股比例（2 表示 1 拆 2）或股票股利比例（0.1 表示 10%）；split/stock_dividend 必填',
  ).optional(),
  amountPerShare: DECIMAL_STRING_SCHEMA.describe('每股现金分红；cash_dividend 必填').optional(),
  currency: CURRENCY_CODE_SCHEMA.describe('现金分红币种；cash_dividend 必填').optional(),
}).describe('单条公司行为');

/** 风控限额。 */
const RISK_LIMITS_SCHEMA = object({
  maxOrderNotional: MONEY_SCHEMA.describe('单笔订单最大名义价值；缺省不限制').optional(),
  maxPositionQuantity:
    DECIMAL_STRING_SCHEMA.describe('单标的最大绝对持仓数量；缺省不限制').optional(),
  maxPositionNotional: MONEY_SCHEMA.describe('单标的最大绝对持仓名义价值；缺省不限制').optional(),
  allowedInstrumentKeys: array(INSTRUMENT_KEY_SCHEMA, { maxLength: 100 })
    .describe('允许交易的标的 key 白名单；缺省不限制')
    .optional(),
}).describe('下单风控限额');

/**
 * 创建金融分析类工具。
 *
 * @param options - 标的与货币解析配置
 * @returns 只读工具定义数组
 */
export function createFinanceAnalysisTools(
  options: FinanceToolOptions = {},
): readonly ToolDefinition[] {
  const context = createToolContext(options);
  return Object.freeze([
    createTool({
      name: 'finance_money_calculate',
      description: '对同币种金额做加/减/乘/除/比较/格式化；金额与结果使用 Decimal 字符串。',
      inputSchema: object({
        operation: MONEY_OPERATION_SCHEMA.describe('要执行的金额运算'),
        left: DECIMAL_STRING_SCHEMA.describe('左操作数金额，十进制字符串'),
        right: DECIMAL_STRING_SCHEMA.describe(
          '右操作数；multiply 为系数、divide 为除数、compare/add/subtract 为金额',
        ).optional(),
        currency: CURRENCY_CODE_SCHEMA.describe('金额币种代码，例如 USD、USDT'),
        decimals: number({ isInteger: true, min: 0, max: 18 })
          .describe('可选货币精度覆盖；缺省使用工具配置或货币默认值')
          .optional(),
      }),
      execute: ({ operation, left, right, currency, decimals }) => {
        const resolved =
          decimals === undefined
            ? context.resolveCurrency(currency)
            : createCurrency(currency, decimals);
        const leftMoney = parseMoney(left, resolved);
        if (operation === 'format') {
          return toolResult({
            result: leftMoney.amount.toString(),
            formatted: formatMoney(leftMoney),
          });
        }
        if (right === undefined) {
          throw new FinanceToolError(`操作 ${operation} 需要 right 参数`);
        }
        const rightDecimal = Decimal.parse(right);
        switch (operation) {
          case 'add':
            return moneyResult(addMoney(leftMoney, parseMoney(right, resolved)));
          case 'subtract':
            return moneyResult(subtractMoney(leftMoney, parseMoney(right, resolved)));
          case 'multiply':
            return moneyResult(multiplyMoney(leftMoney, rightDecimal));
          case 'divide':
            return moneyResult(divideMoney(leftMoney, rightDecimal));
          case 'compare': {
            const rightMoney = parseMoney(right, resolved);
            return toolResult({
              comparison: compareMoney(leftMoney, rightMoney),
              result: leftMoney.amount.toString(),
            });
          }
          default: {
            const exhaustive: never = operation;
            throw new FinanceToolError(`未知金额操作: ${String(exhaustive)}`);
          }
        }
      },
    }),

    createTool({
      name: 'finance_option_price',
      description: 'Black-Scholes 期权定价与 Greeks（模型数值，非结算金额）。',
      inputSchema: object({
        spot: number({ min: 0 }).describe('标的现价'),
        strike: number({ min: 0 }).describe('期权行权价'),
        timeToExpiryYears: number({ min: 0 }).describe('到期时间（年），0 表示已到期'),
        volatility: number({ min: 0 }).describe('年化波动率，0.2 表示 20%'),
        riskFreeRate: number().describe('年化无风险利率，0.05 表示 5%'),
        dividendYield: number().describe('年化分红率，0.02 表示 2%').optional(),
        optionType: OPTION_TYPE_SCHEMA.describe('call 看涨 / put 看跌'),
      }),
      execute: (input) => {
        const result = blackScholes({
          spot: input.spot,
          strike: input.strike,
          timeToExpiryYears: input.timeToExpiryYears,
          volatility: input.volatility,
          riskFreeRate: input.riskFreeRate,
          ...(input.dividendYield === undefined ? {} : { dividendYield: input.dividendYield }),
          optionType: input.optionType as OptionKind,
        });
        return toolResult(result);
      },
    }),

    createTool({
      name: 'finance_option_implied_volatility',
      description: '由期权市场价格反解隐含波动率。',
      inputSchema: object({
        spot: number({ min: 0 }).describe('标的现价'),
        strike: number({ min: 0 }).describe('期权行权价'),
        timeToExpiryYears: number({ min: 0 }).describe('到期时间（年）'),
        riskFreeRate: number().describe('年化无风险利率，0.05 表示 5%'),
        dividendYield: number().describe('年化分红率，0.02 表示 2%').optional(),
        marketPrice: number({ min: 0 }).describe('期权市场价格（每单位标的）'),
        optionType: OPTION_TYPE_SCHEMA.describe('call 看涨 / put 看跌'),
      }),
      execute: (input) => {
        const volatility = impliedVolatility(
          {
            spot: input.spot,
            strike: input.strike,
            timeToExpiryYears: input.timeToExpiryYears,
            riskFreeRate: input.riskFreeRate,
            ...(input.dividendYield === undefined ? {} : { dividendYield: input.dividendYield }),
            optionType: input.optionType as OptionKind,
          },
          input.marketPrice,
        );
        return toolResult({ volatility });
      },
    }),

    createTool({
      name: 'finance_bond_analyze',
      description: '根据债券规格计算票息、应计利息、净价/全价、YTM、久期、凸性与 DV01。',
      inputSchema: object({
        faceValue: DECIMAL_STRING_SCHEMA.describe('债券面值，例如 "1000"'),
        couponRate: DECIMAL_STRING_SCHEMA.describe('年化票面利率，0.05 表示 5%'),
        couponFrequency: oneOf([1, 2, 4, 12] as const).describe('每年付息次数：1/2/4/12'),
        issueDate: DATE_SCHEMA.describe('发行日，YYYY-MM-DD'),
        maturityDate: DATE_SCHEMA.describe('到期日，YYYY-MM-DD'),
        dayCount: DAY_COUNT_SCHEMA.describe('日计数规则'),
        currency: CURRENCY_CODE_SCHEMA.describe('债券计价货币，例如 USD'),
        settlementDate: DATE_SCHEMA.describe('结算日，缺省为发行日').optional(),
        yieldRate: DECIMAL_STRING_SCHEMA.describe(
          '年化到期收益率；与 cleanPrice 至少提供一个',
        ).optional(),
        cleanPrice:
          DECIMAL_STRING_SCHEMA.describe('债券净价；与 yieldRate 至少提供一个').optional(),
      }),
      execute: (input) => {
        if (input.yieldRate === undefined && input.cleanPrice === undefined) {
          throw new FinanceToolError('yieldRate 与 cleanPrice 至少提供一个');
        }
        const currency = context.resolveCurrency(input.currency);
        const bond = createBondSpec({
          id: 'tool-bond',
          faceValue: Decimal.parse(input.faceValue),
          couponRate: Decimal.parse(input.couponRate),
          couponFrequency: input.couponFrequency,
          issueDateMs: parseDateUtc(input.issueDate),
          maturityDateMs: parseDateUtc(input.maturityDate),
          dayCount: input.dayCount,
          currency,
        });
        const settlementDateMs =
          input.settlementDate === undefined
            ? bond.issueDateMs
            : parseDateUtc(input.settlementDate);
        const yieldRate =
          input.yieldRate === undefined ? undefined : Decimal.parse(input.yieldRate);
        const cleanPrice =
          yieldRate === undefined
            ? createMoney(Decimal.parse(input.cleanPrice ?? '0'), currency)
            : bondCleanPrice(bond, settlementDateMs, yieldRate);
        const effectiveYield = yieldRate ?? bondYield(bond, settlementDateMs, cleanPrice);
        return toolResult({
          faceValue: bond.faceValue,
          couponPerPeriod: couponPerPeriod(bond),
          accruedInterest: accruedInterest(bond, settlementDateMs),
          cleanPrice,
          dirtyPrice: bondDirtyPrice(bond, settlementDateMs, effectiveYield),
          yieldRate: effectiveYield,
          macaulayDuration: macaulayDuration(bond, settlementDateMs, effectiveYield),
          modifiedDuration: modifiedDuration(bond, settlementDateMs, effectiveYield),
          convexity: convexity(bond, settlementDateMs, effectiveYield),
          dv01: dv01(bond, settlementDateMs, effectiveYield),
        });
      },
    }),

    createTool({
      name: 'finance_performance_metrics',
      description: '根据价格序列计算简单收益、年化收益/波动率、Sharpe、最大回撤、VaR 与 ES。',
      inputSchema: object({
        prices: array(number(), { minLength: 2, maxLength: 5_000 }).describe(
          '按时间升序的价格序列，至少 2 个正数',
        ),
        periodsPerYear: POSITIVE_INTEGER_SCHEMA.describe('每年期数，例如日频 252、月频 12'),
        riskFreeRate: number().describe('年化无风险利率，0.02 表示 2%；缺省 0').optional(),
      }),
      execute: ({ prices, periodsPerYear, riskFreeRate = 0 }) => {
        const returns = simpleReturns(prices);
        const volatility = annualizedVolatility(returns, periodsPerYear);
        return toolResult({
          returns,
          cumulativeReturn: returns.reduce((total, value) => total * (1 + value), 1) - 1,
          annualizedReturn: annualizedReturn(returns, periodsPerYear),
          annualizedVolatility: volatility,
          sharpeRatio:
            Math.abs(volatility) < 1e-12
              ? null
              : sharpeRatio(returns, riskFreeRate, periodsPerYear),
          maxDrawdown: maxDrawdown(prices),
          historicalVaR95: historicalVaR(returns, 0.95),
          expectedShortfall95: expectedShortfall(returns, 0.95),
        });
      },
    }),

    createTool({
      name: 'finance_corporate_action_adjust',
      description: '按拆股/股票股利/现金分红调整历史价格（现金分红不改变价格）。',
      inputSchema: object({
        prices: array(PRICE_POINT_SCHEMA, { minLength: 1, maxLength: 5_000 }).describe(
          '历史价格点序列，按时间升序',
        ),
        actions: array(CORPORATE_ACTION_SCHEMA, { maxLength: 100 }).describe(
          '公司行为列表；现金分红不改变价格',
        ),
      }),
      execute: ({ prices, actions }) => {
        const parsedActions = actions.map((action, index) =>
          toCorporateAction(context, action, index),
        );
        const adjusted = adjustHistoricPrices(
          prices.map((point) => ({
            timestampMs: point.timestampMs,
            price: Decimal.parse(point.price),
          })),
          parsedActions,
        );
        return toolResult(adjusted);
      },
    }),

    createTool({
      name: 'finance_position_pnl',
      description: '按成交序列重放持仓，并可选择 markPrice 计算未实现/总盈亏与名义价值。',
      inputSchema: object({
        instrument: INSTRUMENT_KEY_SCHEMA.describe('标的 key，例如 binance:BTC/USDT'),
        fills: array(FILL_INPUT_SCHEMA, { minLength: 1, maxLength: 1_000 }).describe(
          '成交序列；时间必须非降序，正负仓位由 side 决定',
        ),
        markPrice:
          DECIMAL_STRING_SCHEMA.describe('可选标记价格；提供时返回未实现/总盈亏').optional(),
      }),
      execute: ({ instrument, fills, markPrice }) => {
        const resolved = context.resolveInstrument(instrument);
        let position = createPosition(resolved, 0);
        for (let index = 0; index < fills.length; index += 1) {
          const fill = fills[index];
          if (fill === undefined) continue;
          const fee =
            fill.fee === undefined
              ? undefined
              : createMoney(
                  Decimal.parse(fill.fee.amount),
                  context.resolveCurrency(fill.fee.currency),
                );
          const created = createFill({
            id: `tool-fill-${index}`,
            orderId: 'tool-order',
            instrument: resolved,
            side: fill.side === 'buy' ? OrderSide.Buy : OrderSide.Sell,
            price: Decimal.parse(fill.price),
            quantity: Decimal.parse(fill.quantity),
            timestampMs: fill.timestampMs,
            ...(fee === undefined ? {} : { fee }),
          });
          position = applyFillToPosition(position, created).position;
        }
        if (markPrice === undefined) return toolResult({ position });
        return toolResult(positionMarkToMarket(position, Decimal.parse(markPrice)));
      },
    }),

    createTool({
      name: 'finance_risk_check',
      description: '按单笔订单/单标的上限检查数量、价格、名义价值与持仓限额。',
      inputSchema: object({
        instrument: INSTRUMENT_KEY_SCHEMA.describe('标的 key，例如 binance:BTC/USDT'),
        side: SIDE_SCHEMA.describe('订单方向：buy 买入 / sell 卖出'),
        quantity: DECIMAL_STRING_SCHEMA.describe('订单数量，必须满足标的 lot/min/max 约束'),
        price: DECIMAL_STRING_SCHEMA.describe('参考价格，必须满足标的 tickSize'),
        limits: RISK_LIMITS_SCHEMA.describe('风控限额；未提供的限额不检查'),
      }),
      execute: ({ instrument, side, quantity, price, limits }) => {
        const resolved = context.resolveInstrument(instrument);
        const riskLimits: RiskLimits = {
          ...(limits.maxOrderNotional === undefined
            ? {}
            : {
                maxOrderNotional: moneyFromInput(
                  context,
                  limits.maxOrderNotional.amount,
                  limits.maxOrderNotional.currency,
                ),
              }),
          ...(limits.maxPositionQuantity === undefined
            ? {}
            : { maxPositionQuantity: Decimal.parse(limits.maxPositionQuantity) }),
          ...(limits.maxPositionNotional === undefined
            ? {}
            : {
                maxPositionNotional: moneyFromInput(
                  context,
                  limits.maxPositionNotional.amount,
                  limits.maxPositionNotional.currency,
                ),
              }),
          ...(limits.allowedInstrumentKeys === undefined
            ? {}
            : { allowedInstrumentKeys: limits.allowedInstrumentKeys }),
        };
        return toolResult(
          checkOrderRisk({
            instrument: resolved,
            side: side === 'buy' ? OrderSide.Buy : OrderSide.Sell,
            quantity: Decimal.parse(quantity),
            price: Decimal.parse(price),
            limits: riskLimits,
          }),
        );
      },
    }),
  ]);
}

/** 构造金额工具结果。 */
function moneyResult(money: ReturnType<typeof parseMoney>) {
  return toolResult({
    result: money.amount.toString(),
    formatted: formatMoney(money),
    currency: money.currency,
  });
}

/** 解析企业行为。 */
function toCorporateAction(
  context: ReturnType<typeof createToolContext>,
  action: {
    readonly type: 'split' | 'stock_dividend' | 'cash_dividend';
    readonly exDateMs: number;
    readonly ratio: string | undefined;
    readonly amountPerShare: string | undefined;
    readonly currency: string | undefined;
  },
  index: number,
): CorporateAction {
  if (action.type === 'cash_dividend') {
    if (action.amountPerShare === undefined || action.currency === undefined) {
      throw new FinanceToolError(`actions[${index}] cash_dividend 需要 amountPerShare 与 currency`);
    }
    return createCashDividend(
      createMoney(Decimal.parse(action.amountPerShare), context.resolveCurrency(action.currency)),
      action.exDateMs,
    );
  }
  if (action.ratio === undefined) {
    throw new FinanceToolError(`actions[${index}] ${action.type} 需要 ratio`);
  }
  const ratio = Decimal.parse(action.ratio);
  return action.type === 'split'
    ? createStockSplit(ratio, action.exDateMs)
    : createStockDividend(ratio, action.exDateMs);
}

/** 从工具输入构造 Money。 */
function moneyFromInput(
  context: ReturnType<typeof createToolContext>,
  amount: string,
  currency: string,
) {
  return createMoney(Decimal.parse(amount), context.resolveCurrency(currency));
}
