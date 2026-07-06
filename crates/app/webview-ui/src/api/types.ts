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
  | 'skipped'
  | 'manual'
  | 'failed'
  | 'withdrawn'

export interface MeResponse {
  username: string
  role: Role
  groups: string[]
  expires_at: number
}

export interface UserNotificationVariableInfo {
  key: string
  label: string
  description: string
  example: string
}

export interface UserNotificationTemplate {
  enabled: boolean
  include_post_tags: boolean
  text_template: string
  tags: string[]
  images: string[]
}

export interface MappingEntry {
  key: string
  value: string
}

export interface TagValueMappingGroup {
  tag: string
  mappings: TagValueMappingEntry[]
}

export interface TagValueMappingEntry {
  source: string
  target: string
}

export type AgentCommandQueueInsertPosition = 'before' | 'after'

export type AgentCommandShortcutScope = 'review' | 'global'

export type AgentCommandTrigger = 'private_command' | 'submission_received'

export type MediaKind = 'Image' | 'Video' | 'File' | 'Audio' | 'Other' | 'Sticker'

export type BlockKindFilter =
  | 'paragraph'
  | 'reply'
  | 'poke'
  | 'json_card'
  | 'forward'
  | {
      attachment: {
        media_kind: MediaKind | null
      }
    }

export type TextMatcher =
  | {
      mode: 'contains'
      needle: string
    }
  | {
      mode: 'starts_with'
      prefix: string
    }
  | {
      mode: 'regex'
      pattern: string
    }

export type IndexFilter =
  | {
      mode: 'nth'
      n: number
    }
  | {
      mode: 'range'
      start: number
      end: number
    }
  | {
      mode: 'first'
    }
  | {
      mode: 'last'
    }

export interface BlockSelector {
  kinds?: BlockKindFilter[] | null
  text?: TextMatcher | null
  index?: IndexFilter | null
}

export type PositionSpec =
  | {
      pos: 'front'
    }
  | {
      pos: 'back'
    }
  | {
      pos: 'index'
      n: number
    }
  | {
      pos: 'before'
      selector: BlockSelector
    }
  | {
      pos: 'after'
      selector: BlockSelector
    }

export type DraftTransform = {
  kind: 'move_blocks'
  selector: BlockSelector
  position: PositionSpec
}

export type RuleCondition =
  | {
      kind: 'all'
      conditions: RuleCondition[]
    }
  | {
      kind: 'any'
      conditions: RuleCondition[]
    }
  | {
      kind: 'not'
      condition: RuleCondition
    }
  | {
      kind: 'has_block'
      selector: BlockSelector
    }
  | {
      kind: 'block_count_at_least'
      selector: BlockSelector
      n: number
    }
  | {
      kind: 'block_count_equals'
      selector: BlockSelector
      n: number
    }

export type AgentCommandPostTarget =
  | {
      target: 'triggering_post'
    }
  | {
      target: 'review_code'
      template: string
    }

export type AgentCommandReviewAction =
  | {
      action: 'approve'
    }
  | {
      action: 'reject'
    }
  | {
      action: 'delete'
    }
  | {
      action: 'defer'
      delay_ms: string
    }
  | {
      action: 'skip'
    }
  | {
      action: 'immediate'
    }
  | {
      action: 'refresh'
    }
  | {
      action: 'rerender'
    }
  | {
      action: 'select_all_messages'
    }
  | {
      action: 'toggle_anonymous'
    }
  | {
      action: 'expand_audit'
    }
  | {
      action: 'show'
    }
  | {
      action: 'comment'
      text_template: string
    }
  | {
      action: 'reply'
      text_template: string
    }
  | {
      action: 'blacklist'
      reason_template: string
    }
  | {
      action: 'quick_reply'
      key_template: string
    }
  | {
      action: 'merge'
      target_review_code: string
    }

export type AgentCommandGlobalAction =
  | {
      action: 'help'
    }
  | {
      action: 'recall'
      review_code: string
    }
  | {
      action: 'withdraw'
      review_code: string
    }
  | {
      action: 'info'
      review_code: string
    }
  | {
      action: 'manual_relogin'
    }
  | {
      action: 'auto_relogin'
    }
  | {
      action: 'pending_list'
    }
  | {
      action: 'pending_clear'
    }
  | {
      action: 'send_queue_clear'
    }
  | {
      action: 'send_queue_flush'
    }
  | {
      action: 'send_in_flight_clear'
    }
  | {
      action: 'blacklist_list'
    }
  | {
      action: 'blacklist_add'
      sender_id: string
      reason_template: string
    }
  | {
      action: 'blacklist_remove'
      sender_id: string
    }
  | {
      action: 'set_external_number'
      value_template: string
    }
  | {
      action: 'quick_reply_list'
    }
  | {
      action: 'quick_reply_add'
      key_template: string
      text_template: string
    }
  | {
      action: 'quick_reply_delete'
      key_template: string
    }
  | {
      action: 'shortcut_list'
    }
  | {
      action: 'shortcut_add'
      scope: AgentCommandShortcutScope
      key_template: string
      definition_template: string
    }
  | {
      action: 'shortcut_delete'
      scope: AgentCommandShortcutScope
      key_template: string
    }
  | {
      action: 'self_check'
    }
  | {
      action: 'system_repair'
    }

export type AgentCommandBlock =
  | {
      kind: 'reply_private_message'
      text_template: string
      tags: string[]
      images: string[]
    }
  | {
      kind: 'start_submission_session'
    }
  | {
      kind: 'finish_submission_session'
    }
  | {
      kind: 'resume_submission_session'
    }
  | {
      kind: 'submit_submission_session'
    }
  | {
      kind: 'cancel_submission_session'
    }
  | {
      kind: 'insert_queued_post'
      moving_post_code: string
      anchor_post_code: string
      position: AgentCommandQueueInsertPosition
    }
  | {
      kind: 'execute_review_action'
      review_code: string
      action: AgentCommandReviewAction
    }
  | {
      kind: 'execute_global_action'
      action: AgentCommandGlobalAction
    }
  | {
      kind: 'if'
      condition: RuleCondition
      then_blocks: AgentCommandBlock[]
      else_blocks: AgentCommandBlock[]
    }
  | {
      kind: 'set_draft_transforms'
      target: AgentCommandPostTarget
      transforms: DraftTransform[]
    }
  | {
      kind: 'send_webhook'
      url: string
      source_webhook: string
      text_template: string
      tags: string[]
      images: string[]
    }

export interface AppConfigAgentCommand {
  name: string
  enabled: boolean
  admin_only: boolean
  trigger: AgentCommandTrigger
  description: string
  blocks: AgentCommandBlock[]
}

export interface ConfigAdminEntry {
  id: string
  username: string
  password: string
  password_set: boolean
}

export interface AppConfigCommonSettings {
  web_api_enabled: boolean
  web_api_port: number
  web_api_root_token: string
  webview_enabled: boolean
  webview_host: string
  webview_port: number
  webview_session_ttl_sec: number
  telemetry_enabled: boolean
  telemetry_local_dir: string
  telemetry_upload_enabled: boolean
  telemetry_upload_interval_sec: number
  telemetry_upload_batch_size: number
  telemetry_max_append_messages: number
  process_waittime_sec: number
  min_interval_ms: number
  max_image_number_one_post: number
  send_timeout_ms: number
  send_max_attempts: number
  tz_offset_minutes: number
  max_cache_mb: number
  napcat_base_url: string
  napcat_access_token: string
  at_unprived_sender: boolean
  friend_request_window_sec: number
  friend_add_message: string
}

export interface AppConfigGroupSettings {
  group_id: string
  audit_group_id: string
  accounts: string[]
  napcat_base_url: string
  napcat_access_token: string
  process_waittime_sec: number
  min_interval_ms: number
  max_post_stack: number
  max_image_number_one_post: number
  send_timeout_ms: number
  send_max_attempts: number
  send_schedule: string[]
  individual_image_in_posts: boolean
  watermark_text: string
  friend_add_message: string
  friend_request_window_sec: number
  quick_replies: MappingEntry[]
  review_shortcuts: MappingEntry[]
  global_shortcuts: MappingEntry[]
  agent_commands: AppConfigAgentCommand[]
  agent_command_admins: string[]
  webview_admins: ConfigAdminEntry[]
}

export interface AppConfigSettingsResponse {
  config_path: string
  common: AppConfigCommonSettings
  global_admins: ConfigAdminEntry[]
  groups: AppConfigGroupSettings[]
  agent_command_variables: UserNotificationVariableInfo[]
}

export interface UserNotificationSettingsResponse {
  group_id: string
  available_groups: string[]
  queue_entered: UserNotificationTemplate
  review_queued: UserNotificationTemplate
  send_succeeded: UserNotificationTemplate
  rejected: UserNotificationTemplate
  webhook_tag_map: MappingEntry[]
  tag_value_map: MappingEntry[]
  tag_value_maps: TagValueMappingGroup[]
  variables: UserNotificationVariableInfo[]
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
  deleted: '已删除',
  skipped: '已跳过',
  manual: '人工处理',
  failed: '失败',
  withdrawn: '已撤回',
}

export const ACTION_LABELS: Record<string, string> = {
  approve: '通过',
  reject: '拒绝',
  delete: '删除',
  defer: '暂缓',
  skip: '跳过',
  immediate: '立即',
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
