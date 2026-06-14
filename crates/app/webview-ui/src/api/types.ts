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
  | 'deleted'
  | 'withdrawn'
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
    deleted: number
  }>
  hourly_distribution: Array<{
    hour: number
    count: number
  }>
  avg_review_time_ms: number | null
}

export interface GroupHealthItem {
  group_id: string
  total_count: number
  pending_count: number
  actionable_count: number
  error_count: number
  failed_count: number
  sent_count: number
  today_count: number
  avg_review_time_ms: number | null
  last_created_at_ms: number | null
}

export interface GroupHealthResponse {
  items: GroupHealthItem[]
}

export interface FailureSummary {
  total_count: number
  stage_failed_count: number
  post_error_count: number
  render_error_count: number
  review_publish_error_count: number
}

export interface FailureItem {
  post_id: string
  review_id: string | null
  review_code: number | null
  group_id: string
  stage: Stage
  source: string
  error: string
  created_at_ms: number
  sender_id: string | null
  preview_text: string | null
}

export interface FailureListResponse {
  summary: FailureSummary
  items: FailureItem[]
}

export interface BlacklistItem {
  group_id: string
  sender_id: string
  reason: string | null
}

export interface BlacklistListResponse {
  items: BlacklistItem[]
  total: number
}

export interface PostTimelineItem {
  label: string
  status: string
  at_ms: number | null
  detail: string | null
}

export interface ListPostsResponse {
  items: PostItem[]
  next_cursor: number | null
  total: number
}

export interface PostCollectionResponse {
  items: PostItem[]
  total: number
}

export interface SimilarPostItem {
  post: PostItem
  similarity_reason: string
}

export interface SimilarPostResponse {
  items: SimilarPostItem[]
  total: number
}

export interface AuditListItem {
  audit_id: string
  operator: string
  action: string
  target_type: string
  target_id: string
  group_id: string | null
  summary: string
  status: string
  created_at_ms: number
}

export interface AuditListResponse {
  items: AuditListItem[]
}

export interface SavedFilterQuery {
  stage?: string | null
  keyword?: string | null
  group_id?: string | null
  sort_by?: string | null
  sort_order?: string | null
  only_error: boolean
  only_actionable: boolean
  page_size?: number | null
}

export interface SavedFilterPreset {
  preset_id: string
  name: string
  query: SavedFilterQuery
  created_at_ms: number
  updated_at_ms: number
}

export interface SavedFilterListResponse {
  items: SavedFilterPreset[]
}

export interface ListReviewIdsResponse {
  review_ids: string[]
  total: number
}

export interface PostDetail {
  post_id: string
  review_id: string | null
  review_code: number | null
  decision_reason: string | null
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
  timeline: PostTimelineItem[]
  sender_name: string | null
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
  deleted: '已删除',
  withdrawn: '已撤回',
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
