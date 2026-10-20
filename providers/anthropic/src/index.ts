/**
 * @file `@kurumi/anthropic` 主入口。
 * @description Anthropic Messages 协议模型适配器；桶导出，不承载业务逻辑。
 * @module anthropic
 */

export { AnthropicProviderError, type AnthropicProviderErrorOptions } from './errors.js';
export {
  DEFAULT_ANTHROPIC_BASE_URL,
  DEFAULT_ANTHROPIC_MAX_TOKENS,
  DEFAULT_ANTHROPIC_PROVIDER_ID,
  DEFAULT_ANTHROPIC_THINKING_BUDGET_TOKENS,
  DEFAULT_ANTHROPIC_VERSION,
  type AnthropicProviderOptions,
} from './options.js';
export { createAnthropicProvider } from './provider.js';
