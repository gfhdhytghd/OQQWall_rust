import type { ApiErrorBody } from './types'

export async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers ?? {}),
    },
    ...init,
  })
  if (!res.ok) {
    let message = `请求失败: ${res.status}`
    try {
      const body = (await res.json()) as ApiErrorBody
      if (body?.error?.message) message = body.error.message
    } catch {
      // ignore
    }
    throw new Error(message)
  }
  if (res.status === 204) return {} as T
  return (await res.json()) as T
}
