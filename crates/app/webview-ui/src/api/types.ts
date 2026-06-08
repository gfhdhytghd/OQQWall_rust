export type Role = 'global_admin' | 'group_admin'

export type Stage =
  | '__active__'
  | ''
  | 'drafted'
  | 'render_requested'
  | 'rendered'
  | 'review_pending'
  | 'reviewed'
  | 'scheduled'
  | 'sending'
  | 'sent'
  | 'rejected'
  | 'skipped'
  | 'manual'
  | 'failed'

export interface MeResponse {
  username: string
  role: Role
  groups: string[]
  expires_at: number
}

export interface PostItem {
  post_id: string
  review_id: string | null
  group_id: string
  stage: Stage
  external_code: number | null
  internal_code: number | null
  sender_id: string | null
  created_at_ms: number
  last_error: string | null
  preview_text?: string
  preview_image_url?: string
  preview_image_urls?: string[]
  preview_image_count?: number
}

export interface StatsResponse {
  pending_count: number
  today_count: number
  total_count: number
  stage_breakdown: Record<string, number>
  actionable_count: number
  error_count: number
  daily_trend: Array<{
    date: string
    submitted: number
    approved: number
    rejected: number
  }>
  hourly_distribution: Array<{
    hour: number
    count: number
  }>
  avg_review_time_ms: number | null
}

export interface ListPostsResponse {
  items: PostItem[]
  next_cursor: number | null
  total: number
}

export interface ListReviewIdsResponse {
  review_ids: string[]
  total: number
}

export interface PostDetail {
  post_id: string
  review_id: string | null
  review_code: number | null
  group_id: string
  stage: Stage
  external_code: number | null
  sender_id: string | null
  session_id: string
  created_at_ms: number
  is_anonymous: boolean
  is_safe: boolean
  blocks: Array<
    | { kind: 'text'; text: string }
    | {
        kind: 'attachment'
        media_kind: string
        reference_type: 'blob_id' | 'remote_url'
        reference: string
        size_bytes: number | null
      }
  >
  render_png_blob_id: string | null
  last_error: string | null
}

export interface ApiErrorBody {
  error?: {
    message?: string
  }
}

export const STAGE_LABELS: Record<string, string> = {
  __active__: '全部活跃',
  '': '全部',
  drafted: '已接收',
  render_requested: '待渲染',
  rendered: '已渲染',
  review_pending: '待审核',
  reviewed: '已审核',
  scheduled: '已排队',
  sending: '发送中',
  sent: '已发送',
  rejected: '已拒绝',
  skipped: '已跳过',
  manual: '人工处理',
  failed: '失败',
}

export const ACTION_LABELS: Record<string, string> = {
  approve: '通过',
  reject: '拒绝',
  delete: '删除',
  defer: '暂缓',
  skip: '跳过',
  immediate: '立即发送',
  refresh: '刷新',
  rerender: '重渲染',
  select_all: '全选',
  toggle_anonymous: '切换匿名',
  expand_audit: '展开审核',
  show: '展示',
  comment: '评论',
  reply: '回复',
  blacklist: '拉黑',
  quick_reply: '快捷回复',
  merge: '合并',
}

export const ACTIONS = [
  'approve',
  'reject',
  'delete',
  'defer',
  'skip',
  'immediate',
  'refresh',
  'rerender',
  'select_all',
  'toggle_anonymous',
  'expand_audit',
  'show',
  'comment',
  'reply',
  'blacklist',
  'quick_reply',
  'merge',
]
