/**
 * @file Kurumi 跨包错误基类与通用错误类型。
 * @description 提供稳定 `code`、`cause` 因果链与正确的子类 `name`。
 * @module core/errors
 */

/** Kurumi 错误构造选项。 */
export interface KurumiErrorOptions {
  /** 稳定错误码，缺省 `kurumi_error`。 */
  readonly code?: string;
  /** 原始错误，保留因果链。 */
  readonly cause?: unknown;
}

/** Kurumi 所有领域错误的基类。 */
export class KurumiError extends Error {
  /** 稳定错误码，程序可据此分支处理。 */
  readonly code: string;

  /**
   * @param message - 面向人类的错误描述
   * @param options - 错误码与 cause
   */
  constructor(message: string, options?: KurumiErrorOptions) {
    super(message, options?.cause === undefined ? undefined : { cause: options.cause });
    this.name = new.target.name;
    this.code = options?.code ?? 'kurumi_error';
  }
}

/** 请求的文件不存在。 */
export class FileNotFoundError extends KurumiError {
  /** 尝试读取的路径。 */
  readonly path: string;

  /**
   * @param path - 不存在的路径
   * @param cause - 底层文件系统错误
   */
  constructor(path: string, cause?: unknown) {
    super(`文件不存在: ${path}`, {
      code: 'file_not_found',
      ...(cause === undefined ? {} : { cause }),
    });
    this.path = path;
  }
}

/** RuntimeHost 错误的构造选项；错误码固定为 `runtime_host_error`。 */
export interface RuntimeHostErrorOptions {
  /** 底层文件系统或宿主异常。 */
  readonly cause?: unknown;
}

/** RuntimeHost 的读写失败。 */
export class RuntimeHostError extends KurumiError {
  /**
   * @param message - 失败描述
   * @param options - 原始 cause
   */
  constructor(message: string, options?: RuntimeHostErrorOptions) {
    super(message, {
      code: 'runtime_host_error',
      ...(options?.cause === undefined ? {} : { cause: options.cause }),
    });
  }
}
