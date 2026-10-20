/**
 * @file RuntimeHost：跨 runtime 的最小能力抽象。
 * @description 核心包不依赖 Node/浏览器 API；文件、环境变量与路径解析由宿主注入。
 * @module core/runtime
 */

/**
 * 运行时宿主。
 *
 * @remarks
 * - `env()` 在每次读取时调用，因此可实现热更新。
 * - `readTextFile()` 对“文件不存在”必须抛出 `FileNotFoundError`，
 *   以便可选文件来源安全跳过；其他错误应包装为 `RuntimeHostError`。
 */
export interface RuntimeHost {
  /** 当前工作目录（或等价基准目录）。 */
  readonly cwd: string;
  /** 返回当前环境变量快照。 */
  env(): Readonly<Record<string, string | undefined>>;
  /** 将 `path` 相对于 `baseDir` 解析为宿主可读的绝对定位字符串。 */
  resolvePath(baseDir: string, path: string): string;
  /**
   * 读取 UTF-8 文本文件。
   *
   * @param path - 宿主可读的绝对定位字符串
   * @param signal - 可选取消信号；实现应尽快响应 abort
   */
  readTextFile(path: string, signal?: AbortSignal): Promise<string>;
}
