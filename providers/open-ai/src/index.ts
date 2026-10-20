/**
 * @file `@kurumi/open-ai` 主入口。
 * @description OpenAI 协议兼容模型适配器；桶导出，不承载业务逻辑。
 * @module open-ai
 */

export { OpenAIProviderError, type OpenAIProviderErrorOptions } from './errors.js';
export {
  DEFAULT_OPENAI_BASE_URL,
  DEFAULT_OPENAI_PROVIDER_ID,
  DEFAULT_REASONING_CONTENT_FIELD,
  DEFAULT_THINKING_FIELD,
  type MaxTokensField,
  type OpenAIProviderOptions,
} from './options.js';
export { createOpenAIProvider } from './provider.js';
