/**
 * @file Provider HTTP 工具单元测试。
 * @description 覆盖 JSON/文本读取、非 2xx、非法 JSON、网络失败、超时与错误体上限。
 * @module finance/provider/http.test
 */

import { describe, expect, it } from 'vitest';

import { ProviderDataError, ProviderHttpError } from './errors.js';
import { fetchJson, fetchText, type FetchLike } from './http.js';

const URL_VALUE = 'https://example.test/data';

function response(body: string | null, status = 200): Response {
  return new Response(body, { status });
}

describe('fetchJson / fetchText', () => {
  it('读取 JSON 与文本', async () => {
    const jsonFetch: FetchLike = async () => response('{"ok":true}');
    await expect(
      fetchJson(URL_VALUE, { providerId: 'test', fetchImpl: jsonFetch }),
    ).resolves.toEqual({ ok: true });

    const textFetch: FetchLike = async () => response('hello');
    await expect(fetchText(URL_VALUE, { providerId: 'test', fetchImpl: textFetch })).resolves.toBe(
      'hello',
    );
  });

  it('非 2xx 抛 ProviderHttpError，并限制错误体长度', async () => {
    const fetchImpl: FetchLike = async () => response('x'.repeat(10_000), 500);
    try {
      await fetchJson(URL_VALUE, { providerId: 'test', fetchImpl });
      throw new Error('expected failure');
    } catch (error) {
      expect(error).toBeInstanceOf(ProviderHttpError);
      expect((error as ProviderHttpError).status).toBe(500);
      expect((error as Error).message.length).toBeLessThan(5_000);
    }
  });

  it('非法 JSON 抛 ProviderDataError', async () => {
    const fetchImpl: FetchLike = async () => response('not-json');
    await expect(fetchJson(URL_VALUE, { providerId: 'test', fetchImpl })).rejects.toBeInstanceOf(
      ProviderDataError,
    );
  });

  it('网络失败抛 status=0 的 ProviderHttpError', async () => {
    const fetchImpl: FetchLike = async () => {
      throw new Error('network down');
    };
    try {
      await fetchJson(URL_VALUE, { providerId: 'test', fetchImpl });
      throw new Error('expected failure');
    } catch (error) {
      expect(error).toBeInstanceOf(ProviderHttpError);
      expect((error as ProviderHttpError).status).toBe(0);
    }
  });

  it('拒绝非法 timeoutMs', async () => {
    const fetchImpl: FetchLike = async () => response('{}');
    await expect(
      fetchJson(URL_VALUE, { providerId: 'test', fetchImpl, timeoutMs: -1 }),
    ).rejects.toBeInstanceOf(ProviderDataError);
    await expect(
      fetchJson(URL_VALUE, { providerId: 'test', fetchImpl, timeoutMs: Number.NaN }),
    ).rejects.toBeInstanceOf(ProviderDataError);
  });

  it('超时抛 ProviderHttpError', async () => {
    const fetchImpl: FetchLike = async (_input, init) =>
      new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => reject(new Error('aborted by timeout')), {
          once: true,
        });
      });

    await expect(
      fetchJson(URL_VALUE, { providerId: 'test', fetchImpl, timeoutMs: 5 }),
    ).rejects.toBeInstanceOf(ProviderHttpError);
  });
});
