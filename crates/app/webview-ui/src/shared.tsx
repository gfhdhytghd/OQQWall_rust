import {
  type ComponentProps,
  type Key,
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { Button, Card, Chip, EmptyState, Input, ListBox, NumberField, Select, Toast, toast } from '@heroui/react'
import {
  Ban,
  Check,
  CheckCircle2,
  Clock3,
  HelpCircle,
  MessageSquare,
  RefreshCcw,
  Send,
  ShieldCheck,
  Trash2,
  X,
  Zap,
} from 'lucide-react'
import {
  ACTION_LABELS,
  AgentCommandBlock,
  AgentCommandGlobalAction,
  AgentCommandQueueInsertPosition,
  AgentCommandReviewAction,
  AgentCommandShortcutScope,
  AppConfigAgentCommand,
  AppConfigSettingsResponse,
  MappingEntry,
  STAGE_LABELS,
  Stage,
  UserNotificationSettingsResponse,
} from './api/types'

export type ViewKey = 'review' | 'sent' | 'stats' | 'settings' | 'tag-mapping' | 'agent'
export type PostsWorkspaceMode = 'review' | 'sent'
export type RuntimeConfigWorkbenchMode = 'full' | 'agent'
export type SettingsTabKey = 'config' | 'notifications'
export type ConfigMenuKey = 'runtime_settings' | 'operation_settings' | 'operation_global_misc'
export type NotificationMenuKey = 'stages' | 'webhooks' | 'variables'
export type TagMappingMenuKey = 'groups' | 'webhooks'
export type PostViewMode = 'cards' | 'list'
export type ToastKind = 'info' | 'success' | 'error'
export type SortOrder = 'asc' | 'desc'
export type SelectOption<T extends string = string> = { value: T; label: string }
export type NotificationStageKey = 'queue_entered' | 'review_queued' | 'send_succeeded' | 'rejected'
export type AgentReviewActionKey =
  | 'approve'
  | 'reject'
  | 'delete'
  | 'defer'
  | 'skip'
  | 'immediate'
  | 'refresh'
  | 'rerender'
  | 'select_all_messages'
  | 'toggle_anonymous'
  | 'expand_audit'
  | 'show'
  | 'comment'
  | 'reply'
  | 'blacklist'
  | 'quick_reply'
  | 'merge'
export type AgentGlobalActionKey =
  | 'help'
  | 'recall'
  | 'withdraw'
  | 'info'
  | 'manual_relogin'
  | 'auto_relogin'
  | 'pending_list'
  | 'pending_clear'
  | 'send_queue_clear'
  | 'send_queue_flush'
  | 'send_in_flight_clear'
  | 'blacklist_list'
  | 'blacklist_add'
  | 'blacklist_remove'
  | 'set_external_number'
  | 'quick_reply_list'
  | 'quick_reply_add'
  | 'quick_reply_delete'
  | 'shortcut_list'
  | 'shortcut_add'
  | 'shortcut_delete'
  | 'self_check'
  | 'system_repair'
export type AgentCommandBlockOptionValue =
  | AgentCommandBlock['kind']
  | `review:${AgentReviewActionKey}`
  | `global:${AgentGlobalActionKey}`
export type SendSuccessReplySettingsResponse = {
  group_id: string
  available_groups: string[]
  enabled: boolean
  text_template: string
  images: string[]
  variables: UserNotificationSettingsResponse['variables']
}
export type PostQuerySnapshot = {
  stage: Stage
  keyword: string
  groupId: string
  sortBy: string
  sortOrder: SortOrder
  page: number
  pageSize: number
  onlyError: boolean
  onlyActionable: boolean
}
export type AgentCommandFieldNode = HTMLInputElement | HTMLTextAreaElement
export type AgentVariableDragPayload = {
  source: 'variable'
  key: string
}
export type ReplyPrivateMessageBlock = Extract<AgentCommandBlock, { kind: 'reply_private_message' }>
export type SendWebhookBlock = Extract<AgentCommandBlock, { kind: 'send_webhook' }>
export type InsertQueuedPostBlock = Extract<AgentCommandBlock, { kind: 'insert_queued_post' }>
export type ExecuteReviewActionBlock = Extract<AgentCommandBlock, { kind: 'execute_review_action' }>
export type ExecuteGlobalActionBlock = Extract<AgentCommandBlock, { kind: 'execute_global_action' }>
export type AgentBlockDragPayload =
  | {
      source: 'palette'
      option: AgentCommandBlockOptionValue
    }
  | {
      source: 'workspace'
      commandIndex: number
      blockIndex: number
    }
  | AgentVariableDragPayload

export type RuntimeConfigPageKey =
  | 'runtime_settings'
  | 'operation_settings'
  | 'operation_global_misc'

export const ACTIVE_EXCLUDED = new Set(['sent', 'rejected', 'deleted', 'skipped', 'failed', 'withdrawn'])
export const PAGE_SIZES = [20, 50, 100, 200]
export const AGENT_BLOCK_DRAG_MIME = 'application/x-oqqwall-agent-block'
export const AGENT_REVIEW_ACTION_OPTIONS: Array<SelectOption<AgentReviewActionKey>> = [
  { value: 'approve', label: '通过稿件' },
  { value: 'reject', label: '拒稿' },
  { value: 'delete', label: '删除稿件' },
  { value: 'defer', label: '暂缓处理' },
  { value: 'skip', label: '跳过' },
  { value: 'immediate', label: '立即' },
  { value: 'refresh', label: '刷新' },
  { value: 'rerender', label: '重渲染' },
  { value: 'select_all_messages', label: '选择全部消息' },
  { value: 'toggle_anonymous', label: '切换匿名' },
  { value: 'expand_audit', label: '展开审核内容' },
  { value: 'show', label: '显示稿件' },
  { value: 'comment', label: '添加评论' },
  { value: 'reply', label: '回复投稿人' },
  { value: 'blacklist', label: '拉黑投稿人' },
  { value: 'quick_reply', label: '执行快捷回复' },
  { value: 'merge', label: '合并到其他审核编号' },
]
export const AGENT_GLOBAL_ACTION_OPTIONS: Array<SelectOption<AgentGlobalActionKey>> = [
  { value: 'help', label: '返回帮助' },
  { value: 'recall', label: '召回已处理稿件' },
  { value: 'withdraw', label: '撤回发送中的稿件' },
  { value: 'info', label: '查询稿件信息' },
  { value: 'manual_relogin', label: '手动重登提示' },
  { value: 'auto_relogin', label: '自动重登提示' },
  { value: 'pending_list', label: '列出待审核稿件' },
  { value: 'pending_clear', label: '清空待审核缓存' },
  { value: 'send_queue_clear', label: '清空发送队列' },
  { value: 'send_queue_flush', label: '立刻冲刷发送队列' },
  { value: 'send_in_flight_clear', label: '清空发送中状态' },
  { value: 'blacklist_list', label: '查看黑名单' },
  { value: 'blacklist_add', label: '添加黑名单' },
  { value: 'blacklist_remove', label: '移出黑名单' },
  { value: 'set_external_number', label: '设置外部编号' },
  { value: 'quick_reply_list', label: '查看快捷回复' },
  { value: 'quick_reply_add', label: '新增快捷回复' },
  { value: 'quick_reply_delete', label: '删除快捷回复' },
  { value: 'shortcut_list', label: '查看快捷指令' },
  { value: 'shortcut_add', label: '新增快捷指令' },
  { value: 'shortcut_delete', label: '删除快捷指令' },
  { value: 'self_check', label: '系统自检' },
  { value: 'system_repair', label: '系统修复提示' },
]
export const AGENT_QUEUE_INSERT_POSITION_OPTIONS: Array<SelectOption<AgentCommandQueueInsertPosition>> = [
  { value: 'before', label: '插到目标稿件之前' },
  { value: 'after', label: '插到目标稿件之后' },
]
export const AGENT_SHORTCUT_SCOPE_OPTIONS: Array<SelectOption<AgentCommandShortcutScope>> = [
  { value: 'review', label: '审核快捷指令' },
  { value: 'global', label: '全局快捷指令' },
]
export const AGENT_COMMAND_BLOCK_OPTIONS: Array<SelectOption<AgentCommandBlockOptionValue>> = [
  { value: 'reply_private_message', label: '消息 | 回复私聊消息' },
  { value: 'start_submission_session', label: '投稿会话 | 开始会话' },
  { value: 'finish_submission_session', label: '投稿会话 | 结束并等待确认' },
  { value: 'resume_submission_session', label: '投稿会话 | 继续编辑' },
  { value: 'submit_submission_session', label: '投稿会话 | 提交会话' },
  { value: 'cancel_submission_session', label: '投稿会话 | 取消会话' },
  { value: 'insert_queued_post', label: '队列 | 调整排队顺序' },
  ...AGENT_REVIEW_ACTION_OPTIONS.map((option) => ({
    value: `review:${option.value}` as AgentCommandBlockOptionValue,
    label: `审核 | ${option.label}`,
  })),
  ...AGENT_GLOBAL_ACTION_OPTIONS.map((option) => ({
    value: `global:${option.value}` as AgentCommandBlockOptionValue,
    label: `系统 | ${option.label}`,
  })),
  { value: 'send_webhook', label: 'Webhook | 发送请求' },
]
export const STAGE_OPTIONS: Array<SelectOption<Stage>> = [
  { value: '__active__', label: '全部活跃' },
  { value: '', label: '全部' },
  { value: 'review_pending', label: '待审核' },
  { value: 'reviewed', label: '已审核' },
  { value: 'scheduled', label: '已排队' },
  { value: 'sending', label: '发送中' },
  { value: 'sent', label: '已发送' },
  { value: 'rejected', label: '已拒绝' },
  { value: 'deleted', label: '已删除' },
  { value: 'skipped', label: '已跳过' },
  { value: 'manual', label: '人工处理' },
  { value: 'failed', label: '失败' },
  { value: 'withdrawn', label: '已撤回' },
]
export const REVIEW_STAGE_OPTIONS = STAGE_OPTIONS.filter((option) => option.value !== 'sent')
export const SORT_OPTIONS: Array<SelectOption> = [
  { value: 'created_at:desc', label: '最新优先' },
  { value: 'created_at:asc', label: '最早优先' },
  { value: 'code:desc', label: '编号优先' },
  { value: 'stage:asc', label: '状态排序' },
]
export const BATCH_ACTIONS = ['approve', 'reject', 'delete', 'skip', 'immediate', 'refresh', 'rerender']
export const DANGEROUS_ACTIONS = new Set(['reject', 'delete', 'blacklist'])
export const LIST_PRIMARY_ACTIONS = ['approve', 'reject', 'delete'] as const
export const CARD_QUICK_ACTIONS = [
  'approve',
  'skip',
  'immediate',
  'reject',
  'delete',
  'blacklist',
  'comment',
  'refresh',
  'rerender',
] as const
export const DETAIL_QUICK_ACTIONS = CARD_QUICK_ACTIONS
export const DETAIL_ACTIONS = [
  'approve',
  'reject',
  'delete',
  'defer',
  'skip',
  'immediate',
  'refresh',
  'rerender',
  'toggle_anonymous',
  'comment',
  'reply',
  'blacklist',
  'quick_reply',
  'merge',
]
export const NOTIFICATION_STAGE_OPTIONS: Array<
  SelectOption<NotificationStageKey> & { icon: ReactNode }
> = [
  {
    value: 'queue_entered',
    label: '进入队列',
    icon: <Clock3 size={16} />,
  },
  {
    value: 'review_queued',
    label: '进入审核队列',
    icon: <ShieldCheck size={16} />,
  },
  {
    value: 'send_succeeded',
    label: '发稿成功',
    icon: <CheckCircle2 size={16} />,
  },
  {
    value: 'rejected',
    label: '拒稿',
    icon: <Ban size={16} />,
  },
]

export function MappingListEditor({
  label,
  hint,
  leftPlaceholder,
  rightPlaceholder,
  entries,
  onAdd,
  onChange,
  onRemove,
}: {
  label: string
  hint?: string
  leftPlaceholder: string
  rightPlaceholder: string
  entries: MappingEntry[]
  onAdd: () => void
  onChange: (index: number, key: keyof MappingEntry, value: string) => void
  onRemove: (index: number) => void
}) {
  return (
    <div className="settings-panel">
      <div className="settings-section-head">
        <span className="field-label">{label}</span>
        <Button size="sm" variant="secondary" onClick={onAdd}>
          新增映射
        </Button>
      </div>
      {hint ? <span className="field-hint">{hint}</span> : null}
      {entries.length ? (
        <div className="settings-variable-list">
          {entries.map((entry, index) => (
            <div key={`${label}-${index}`} className="settings-variable-card">
              <div className="config-mapping-row">
                <Input
                  className="settings-image-input"
                  placeholder={leftPlaceholder}
                  value={entry.key}
                  onChange={(event) => onChange(index, 'key', event.target.value)}
                />
                <Input
                  className="settings-image-input"
                  placeholder={rightPlaceholder}
                  value={entry.value}
                  onChange={(event) => onChange(index, 'value', event.target.value)}
                />
                <Button
                  size="sm"
                  variant="tertiary"
                  isIconOnly
                  aria-label={`删除 ${label} ${index + 1}`}
                  onClick={() => onRemove(index)}
                >
                  <Trash2 size={16} />
                </Button>
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="settings-empty">
          <MessageSquare size={20} />
          <span>当前还没有配置 {label}。</span>
        </div>
      )}
    </div>
  )
}

export function SettingsMenuCard<T extends string>({
  options,
  activeKey,
  onSelect,
}: {
  title: string
  options: Array<{
    value: T
    label: string
    description?: string
    icon?: ReactNode
  }>
  activeKey: T
  onSelect: (value: T) => void
}) {
  return (
    <Card className="panel-card settings-nav-card">
      <Card.Content>
        <div className="settings-nav-list">
          {options.map((option) => (
            <button
              key={option.value}
              type="button"
              className={`settings-nav-button ${activeKey === option.value ? 'is-active' : ''}`}
              onClick={() => onSelect(option.value)}
            >
              <div className="settings-nav-button-top">
                {option.icon ? <span className="settings-stage-icon">{option.icon}</span> : null}
                <strong>{option.label}</strong>
              </div>
              {option.description ? <span>{option.description}</span> : null}
            </button>
          ))}
        </div>
      </Card.Content>
    </Card>
  )
}

export function HeroSelect({
  ariaLabel,
  selectedKey,
  options,
  onSelect,
  className,
}: {
  ariaLabel: string
  selectedKey: string
  options: Array<SelectOption>
  onSelect: (value: string) => void
  className?: string
}) {
  return (
    <Select
      className={className}
      aria-label={ariaLabel}
      selectedKey={selectedKey}
      onSelectionChange={(key: Key | null) => {
        if (key !== null) onSelect(String(key))
      }}
    >
      <Select.Trigger>
        <Select.Value />
        <Select.Indicator />
      </Select.Trigger>
      <Select.Popover>
        <ListBox items={options} aria-label={ariaLabel}>
          {(item) => <ListBox.Item id={item.value}>{item.label}</ListBox.Item>}
        </ListBox>
      </Select.Popover>
    </Select>
  )
}

export function DelayField({
  value,
  onChange,
  className,
}: {
  value: number
  onChange: (value: number) => void
  className?: string
}) {
  return (
    <NumberField className={className} value={value} minValue={1000} step={60000} onChange={onChange} aria-label="延迟毫秒">
      <NumberField.Group>
        <NumberField.DecrementButton>-</NumberField.DecrementButton>
        <NumberField.Input />
        <NumberField.IncrementButton>+</NumberField.IncrementButton>
      </NumberField.Group>
    </NumberField>
  )
}

export function Metric({
  label,
  value,
  tone,
  icon,
}: {
  label: string
  value: number | string
  tone: 'neutral' | 'good' | 'warn' | 'bad'
  icon: React.ReactNode
}) {
  return (
    <Card className={`metric metric-${tone}`} variant="secondary">
      <Card.Content>
        <div className="metric-top">
          <span>{label}</span>
          <div className="metric-icon">{icon}</div>
        </div>
        <strong>{value}</strong>
      </Card.Content>
    </Card>
  )
}

export function StageChip({ stage }: { stage: string }) {
  return (
    <Chip className={`stage-chip stage-chip-${stage}`} variant="soft" size="sm">
      {STAGE_LABELS[stage] ?? stage}
    </Chip>
  )
}

export function quickActionVariant(action: string): ComponentProps<typeof Button>['variant'] {
  if (action === 'reject' || action === 'delete' || action === 'blacklist') return 'danger-soft'
  if (action === 'approve') return undefined
  if (action === 'immediate') return 'secondary'
  return 'secondary'
}

export function cardActionLabel(action: string) {
  if (action === 'skip') return '否'
  if (action === 'immediate') return '立即'
  return ACTION_LABELS[action] ?? action
}

export function quickActionIcon(action: string) {
  if (action === 'approve') return <Check size={16} />
  if (action === 'reject') return <X size={16} />
  if (action === 'delete') return <Trash2 size={16} />
  if (action === 'immediate') return <Zap size={16} />
  if (action === 'blacklist') return <Ban size={16} />
  if (action === 'comment') return <MessageSquare size={16} />
  if (action === 'refresh' || action === 'rerender') return <RefreshCcw size={16} />
  if (action === 'skip') return <HelpCircle size={16} />
  return null
}

export function EmptyPanel({ icon, text }: { icon: React.ReactNode; text: string }) {
  return (
    <EmptyState className="empty-state">
      <div className="empty-icon">{icon}</div>
      <span>{text}</span>
    </EmptyState>
  )
}

export function showToast(kind: ToastKind, text: string) {
  if (kind === 'success') {
    toast.success(text)
  } else if (kind === 'error') {
    toast.danger(text)
  } else {
    toast.info(text)
  }
}

export function useMediaQuery(query: string) {
  const [matches, setMatches] = useState(false)

  useEffect(() => {
    const media = window.matchMedia(query)
    const update = () => setMatches(media.matches)
    update()
    media.addEventListener('change', update)
    return () => media.removeEventListener('change', update)
  }, [query])

  return matches
}

export function useMasonryLayout(dependencies: React.DependencyList) {
  const gridRef = useRef<HTMLDivElement | null>(null)

  useLayoutEffect(() => {
    const grid = gridRef.current
    if (!grid) return
    const gridElement = grid

    let frame = 0
    const rowHeight = Number.parseFloat(window.getComputedStyle(gridElement).gridAutoRows) || 8
    const rowGap = Number.parseFloat(window.getComputedStyle(gridElement).rowGap) || 0

    function measure(card: Element) {
      const target = card.firstElementChild as HTMLElement | null
      const height = Math.ceil((target ?? card).getBoundingClientRect().height)
      const span = Math.max(1, Math.ceil((height + rowGap) / (rowHeight + rowGap)))
      ;(card as HTMLElement).style.gridRowEnd = `span ${span}`
    }

    function layoutAll() {
      window.cancelAnimationFrame(frame)
      frame = window.requestAnimationFrame(() => {
        gridElement.querySelectorAll('.post-card-wrap').forEach(measure)
      })
    }

    const resizeObserver = new ResizeObserver((entries) => {
      window.cancelAnimationFrame(frame)
      frame = window.requestAnimationFrame(() => {
        for (const entry of entries) measure(entry.target)
      })
    })

    gridElement.querySelectorAll('.post-card-wrap').forEach((card) => resizeObserver.observe(card))

    const onImageLoad = (event: Event) => {
      const target = event.target as HTMLElement | null
      if (target?.tagName === 'IMG') layoutAll()
    }
    const onResize = () => layoutAll()

    gridElement.addEventListener('load', onImageLoad, true)
    window.addEventListener('resize', onResize)
    layoutAll()

    return () => {
      window.cancelAnimationFrame(frame)
      resizeObserver.disconnect()
      gridElement.removeEventListener('load', onImageLoad, true)
      window.removeEventListener('resize', onResize)
    }
  }, dependencies)

  return gridRef
}

export function buildReplyImageLabel(value: string, index: number) {
  const trimmed = value.trim()
  if (!trimmed) return `图片 ${index + 1}：未填写地址`
  if (trimmed.startsWith('data:image/')) return `图片 ${index + 1}：已上传`
  return trimmed.length > 72 ? `${trimmed.slice(0, 72)}...` : trimmed
}

export function isPreviewableImageSource(value: string) {
  const trimmed = value.trim().toLowerCase()
  return (
    trimmed.startsWith('data:image/') ||
    trimmed.startsWith('http://') ||
    trimmed.startsWith('https://')
  )
}

export function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      if (typeof reader.result === 'string') {
        resolve(reader.result)
      } else {
        reject(new Error(`无法读取文件：${file.name}`))
      }
    }
    reader.onerror = () => reject(new Error(`无法读取文件：${file.name}`))
    reader.readAsDataURL(file)
  })
}

export function buildPostParams({
  stage,
  keyword,
  groupId,
  sortBy,
  sortOrder,
  page,
  pageSize,
  onlyError,
  onlyActionable,
}: PostQuerySnapshot) {
  const params = new URLSearchParams()
  if (stage && stage !== '__active__') params.set('stage', stage)
  if (stage === '__active__') params.set('active_only', 'true')
  if (keyword.trim()) params.set('keyword', keyword.trim())
  if (groupId) params.set('group_id', groupId)
  if (onlyError) params.set('only_error', 'true')
  if (onlyActionable) params.set('actionable_only', 'true')
  params.set('sort_by', sortBy)
  params.set('sort_order', sortOrder)
  params.set('cursor', String(page * pageSize))
  params.set('limit', String(pageSize))
  return params
}

export function buildActionPayload(action: string, text: string, delayMs: number) {
  const payload: Record<string, unknown> = { action }
  const trimmed = text.trim()
  if (action === 'defer') payload.delay_ms = delayMs
  if (action === 'reject' && trimmed) payload.comment = trimmed
  if (action === 'comment' || action === 'reply') payload.text = trimmed
  if (action === 'blacklist') payload.comment = trimmed
  if (action === 'quick_reply') payload.quick_reply_key = trimmed
  if (action === 'merge') payload.target_review_code = Number(trimmed)
  return payload
}

export function formatDateTime(ms: number) {
  return new Date(ms).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export function formatDuration(ms: number | null) {
  if (!ms) return '-'
  const minutes = Math.round(ms / 60000)
  if (minutes < 60) return `${minutes} 分钟`
  return `${Math.round(minutes / 60)} 小时`
}
