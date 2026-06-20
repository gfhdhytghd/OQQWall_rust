import {
  type ComponentProps,
  type CSSProperties,
  type DragEvent,
  type FormEvent,
  type Key,
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import {
  Button,
  Card,
  Checkbox,
  Chip,
  Drawer,
  EmptyState,
  Input,
  ListBox,
  NumberField,
  Pagination,
  Popover,
  Select,
  Spinner,
  Switch,
  TextArea,
  ToggleButton,
  ToggleButtonGroup,
  Toast,
  toast,
} from '@heroui/react'
import {
  AlertCircle,
  BarChart3,
  Ban,
  Check,
  CheckCircle2,
  Clock3,
  Eye,
  FileText,
  HelpCircle,
  Inbox,
  LayoutGrid,
  List,
  LogOut,
  MessageSquare,
  MoreHorizontal,
  PanelRightOpen,
  RefreshCcw,
  Search,
  Send,
  ShieldCheck,
  Trash2,
  UserRound,
  X,
  Zap,
} from 'lucide-react'
import { api } from './api/client'
import {
  ACTION_LABELS,
  AgentCommandBlock,
  AgentCommandGlobalAction,
  AgentCommandQueueInsertPosition,
  AgentCommandReviewAction,
  AgentCommandShortcutScope,
  AppConfigAgentCommand,
  AppConfigCommonSettings,
  AppConfigGroupSettings,
  AppConfigSettingsResponse,
  ListPostsResponse,
  ListReviewIdsResponse,
  MappingEntry,
  MeResponse,
  PostDetail,
  PostItem,
  STAGE_LABELS,
  Stage,
  StatsResponse,
  ConfigAdminEntry,
  TagValueMappingEntry,
  TagValueMappingGroup,
  UserNotificationSettingsResponse,
  UserNotificationTemplate,
} from './api/types'

type ViewKey = 'review' | 'sent' | 'stats' | 'settings' | 'tag-mapping' | 'agent'
type PostsWorkspaceMode = 'review' | 'sent'
type RuntimeConfigWorkbenchMode = 'full' | 'agent'
type SettingsTabKey = 'config' | 'notifications'
type ConfigMenuKey =
  | 'common_runtime'
  | 'common_services'
  | 'common_telemetry'
  | 'global_admins'
  | 'group_basic'
  | 'group_delivery'
  | 'group_quick_replies'
  | 'group_shortcuts'
  | 'group_agent_commands'
  | 'group_admins'
type NotificationMenuKey = 'stages' | 'webhooks' | 'variables'
type TagMappingMenuKey = 'groups' | 'webhooks'
type PostViewMode = 'cards' | 'list'
type ToastKind = 'info' | 'success' | 'error'
type SortOrder = 'asc' | 'desc'
type SelectOption<T extends string = string> = { value: T; label: string }
type NotificationStageKey = 'queue_entered' | 'review_queued' | 'send_succeeded' | 'rejected'
type AgentReviewActionKey =
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
type AgentGlobalActionKey =
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
type AgentCommandBlockOptionValue =
  | AgentCommandBlock['kind']
  | `review:${AgentReviewActionKey}`
  | `global:${AgentGlobalActionKey}`
type SendSuccessReplySettingsResponse = {
  group_id: string
  available_groups: string[]
  enabled: boolean
  text_template: string
  images: string[]
  variables: UserNotificationSettingsResponse['variables']
}
type PostQuerySnapshot = {
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
type AgentCommandFieldNode = HTMLInputElement | HTMLTextAreaElement
type AgentVariableDragPayload = {
  source: 'variable'
  key: string
}
type ReplyPrivateMessageBlock = Extract<AgentCommandBlock, { kind: 'reply_private_message' }>
type SendWebhookBlock = Extract<AgentCommandBlock, { kind: 'send_webhook' }>
type InsertQueuedPostBlock = Extract<AgentCommandBlock, { kind: 'insert_queued_post' }>
type ExecuteReviewActionBlock = Extract<AgentCommandBlock, { kind: 'execute_review_action' }>
type ExecuteGlobalActionBlock = Extract<AgentCommandBlock, { kind: 'execute_global_action' }>
type AgentBlockDragPayload =
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

type RuntimeConfigPageKey =
  | 'common_runtime'
  | 'common_services'
  | 'common_telemetry'
  | 'global_admins'
  | 'group_basic'
  | 'group_delivery'
  | 'group_quick_replies'
  | 'group_shortcuts'
  | 'group_agent_commands'
  | 'group_admins'

const ACTIVE_EXCLUDED = new Set(['sent', 'rejected', 'skipped', 'failed'])
const PAGE_SIZES = [20, 50, 100, 200]
const AGENT_BLOCK_DRAG_MIME = 'application/x-oqqwall-agent-block'
const AGENT_REVIEW_ACTION_OPTIONS: Array<SelectOption<AgentReviewActionKey>> = [
  { value: 'approve', label: '通过稿件' },
  { value: 'reject', label: '拒稿' },
  { value: 'delete', label: '删除稿件' },
  { value: 'defer', label: '暂缓处理' },
  { value: 'skip', label: '跳过' },
  { value: 'immediate', label: '立即发送' },
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
const AGENT_GLOBAL_ACTION_OPTIONS: Array<SelectOption<AgentGlobalActionKey>> = [
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
const AGENT_QUEUE_INSERT_POSITION_OPTIONS: Array<SelectOption<AgentCommandQueueInsertPosition>> = [
  { value: 'before', label: '插到目标稿件之前' },
  { value: 'after', label: '插到目标稿件之后' },
]
const AGENT_SHORTCUT_SCOPE_OPTIONS: Array<SelectOption<AgentCommandShortcutScope>> = [
  { value: 'review', label: '审核快捷指令' },
  { value: 'global', label: '全局快捷指令' },
]
const AGENT_COMMAND_BLOCK_OPTIONS: Array<SelectOption<AgentCommandBlockOptionValue>> = [
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
const STAGE_OPTIONS: Array<SelectOption<Stage>> = [
  { value: '__active__', label: '全部活跃' },
  { value: '', label: '全部' },
  { value: 'review_pending', label: '待审核' },
  { value: 'reviewed', label: '已审核' },
  { value: 'scheduled', label: '已排队' },
  { value: 'sending', label: '发送中' },
  { value: 'sent', label: '已发送' },
  { value: 'rejected', label: '已拒绝' },
  { value: 'skipped', label: '已跳过' },
  { value: 'manual', label: '人工处理' },
  { value: 'failed', label: '失败' },
]
const REVIEW_STAGE_OPTIONS = STAGE_OPTIONS.filter((option) => option.value !== 'sent')
const SORT_OPTIONS: Array<SelectOption> = [
  { value: 'created_at:desc', label: '最新优先' },
  { value: 'created_at:asc', label: '最早优先' },
  { value: 'code:desc', label: '编号优先' },
  { value: 'stage:asc', label: '状态排序' },
]
const BATCH_ACTIONS = ['approve', 'reject', 'delete', 'skip', 'immediate', 'refresh', 'rerender']
const DANGEROUS_ACTIONS = new Set(['reject', 'delete', 'blacklist'])
const LIST_PRIMARY_ACTIONS = ['approve', 'reject', 'delete'] as const
const CARD_QUICK_ACTIONS = [
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
const DETAIL_QUICK_ACTIONS = CARD_QUICK_ACTIONS
const DETAIL_ACTIONS = [
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
const NOTIFICATION_STAGE_OPTIONS: Array<
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

function App() {
  const [me, setMe] = useState<MeResponse | null>(null)
  const [authChecked, setAuthChecked] = useState(false)
  const [view, setView] = useState<ViewKey>('review')

  useEffect(() => {
    api<MeResponse>('/auth/me')
      .then(setMe)
      .catch(() => setMe(null))
      .finally(() => setAuthChecked(true))
  }, [])

  async function logout() {
    await api('/auth/logout', { method: 'POST' }).catch(() => undefined)
    setMe(null)
  }

  const notify = (kind: ToastKind, text: string) => showToast(kind, text)

  if (!authChecked) {
    return (
      <HeroShell>
        <div className="boot">
          <Spinner />
        </div>
      </HeroShell>
    )
  }

  if (!me) {
    return (
      <HeroShell>
        <LoginView onAuthed={setMe} notify={notify} />
      </HeroShell>
    )
  }

  return (
    <HeroShell>
      <div className="app-shell">
        <aside className="sidebar">
          <Brand />
          <nav className="nav" aria-label="主导航">
            <Button
              className="nav-button"
              variant={view === 'review' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('review')}
            >
              <Eye size={18} />
              审核
            </Button>
            <Button
              className="nav-button"
              variant={view === 'sent' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('sent')}
            >
              <Send size={18} />
              已发送
            </Button>
            <Button
              className="nav-button"
              variant={view === 'stats' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('stats')}
            >
              <BarChart3 size={18} />
              统计
            </Button>
            {me.role === 'global_admin' ? (
              <Button
                className="nav-button"
                variant={view === 'agent' ? 'primary' : 'tertiary'}
                fullWidth
                onClick={() => setView('agent')}
              >
                <FileText size={18} />
                Agent
              </Button>
            ) : null}
            <Button
              className="nav-button"
              variant={view === 'settings' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('settings')}
            >
              <Send size={18} />
              设置
            </Button>
            <Button
              className="nav-button"
              variant={view === 'tag-mapping' ? 'primary' : 'tertiary'}
              fullWidth
              onClick={() => setView('tag-mapping')}
            >
              <LayoutGrid size={18} />
              标签映射
            </Button>
          </nav>
          <Card className="account-card" variant="secondary">
            <Card.Content>
              <div className="account-name">{me.username}</div>
              <div className="account-role">
                {me.role === 'global_admin' ? '全局管理员' : me.groups.join(', ')}
              </div>
              <Button size="sm" variant="secondary" fullWidth onClick={logout}>
                <LogOut size={16} />
                退出
              </Button>
            </Card.Content>
          </Card>
        </aside>

        <main className="main">
          {view === 'review' ? (
            <PostsView
              notify={notify}
              mode="review"
              title="稿件审核"
              description="选择左侧稿件后可在右侧处理详情"
              emptyText="没有符合条件的稿件"
              stageOptions={REVIEW_STAGE_OPTIONS}
              initialStage="__active__"
              allowSelection
              allowActions
              allowBatchActions
              allowOnlyActionable
              allowOnlyError
              allowStageFilter
              allowDetailActions
            />
          ) : view === 'sent' ? (
            <PostsView
              notify={notify}
              mode="sent"
              title="已发送"
              description="这里只展示已经发送完成的稿件，方便单独查看"
              emptyText="没有已发送的稿件"
              stageOptions={[{ value: 'sent', label: '已发送' }]}
              initialStage="sent"
              allowSelection={false}
              allowActions={false}
              allowBatchActions={false}
              allowOnlyActionable={false}
              allowOnlyError={false}
              allowStageFilter={false}
              allowDetailActions={false}
            />
          ) : view === 'stats' ? (
            <StatsView notify={notify} />
          ) : view === 'agent' && me.role === 'global_admin' ? (
            <AgentView notify={notify} />
          ) : view === 'tag-mapping' ? (
            <TagMappingView notify={notify} />
          ) : (
            <SettingsView me={me} notify={notify} />
          )}
        </main>
      </div>
    </HeroShell>
  )
}

function HeroShell({ children }: { children: React.ReactNode }) {
  return (
    <>
      {children}
      <Toast.Provider placement="bottom end" />
    </>
  )
}

function getReviewStageOptions() {
  return STAGE_OPTIONS.filter((option) => option.value !== 'sent')
}

function Brand({ large = false }: { large?: boolean }) {
  return (
    <div className={large ? 'brand brand-large' : 'brand'}>
      <div>
        <strong>OQQWall</strong>
        <span>审核后台</span>
      </div>
    </div>
  )
}

function LoginView({
  onAuthed,
  notify,
}: {
  onAuthed: (me: MeResponse) => void
  notify: (kind: ToastKind, text: string) => void
}) {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [loading, setLoading] = useState(false)

  async function submit(event: FormEvent) {
    event.preventDefault()
    setLoading(true)
    try {
      const result = await api<MeResponse>('/auth/login', {
        method: 'POST',
        body: JSON.stringify({ username, password }),
      })
      onAuthed(result)
      notify('success', '登录成功')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="login-page">
      <Card className="login-card">
        <Card.Content>
          <form className="login-form" onSubmit={submit}>
            <Brand large />
            <Input
              fullWidth
              placeholder="用户名"
              value={username}
              autoComplete="username"
              onChange={(event) => setUsername(event.target.value)}
            />
            <Input
              fullWidth
              placeholder="密码"
              type="password"
              value={password}
              autoComplete="current-password"
              onChange={(event) => setPassword(event.target.value)}
            />
            <Button type="submit" fullWidth isDisabled={loading || !username || !password}>
              {loading ? <Spinner size="sm" /> : <ShieldCheck size={16} />}
              登录
            </Button>
          </form>
        </Card.Content>
      </Card>
    </div>
  )
}

type PostsViewProps = {
  notify: (kind: ToastKind, text: string) => void
  mode: PostsWorkspaceMode
  title: string
  description: string
  emptyText: string
  stageOptions: Array<SelectOption<Stage>>
  initialStage: Stage
  allowSelection: boolean
  allowActions: boolean
  allowBatchActions: boolean
  allowOnlyActionable: boolean
  allowOnlyError: boolean
  allowStageFilter: boolean
  allowDetailActions: boolean
}

function PostsView({
  notify,
  mode,
  title,
  description,
  emptyText,
  stageOptions,
  initialStage,
  allowSelection,
  allowActions,
  allowBatchActions,
  allowOnlyActionable,
  allowOnlyError,
  allowStageFilter,
  allowDetailActions,
}: PostsViewProps) {
  const [posts, setPosts] = useState<PostItem[]>([])
  const [total, setTotal] = useState(0)
  const [loading, setLoading] = useState(false)
  const [actionLoading, setActionLoading] = useState(false)
  const [stage, setStage] = useState<Stage>(initialStage)
  const [keyword, setKeyword] = useState('')
  const [groupId, setGroupId] = useState('')
  const [sortBy, setSortBy] = useState('created_at')
  const [sortOrder, setSortOrder] = useState<SortOrder>('desc')
  const [page, setPage] = useState(0)
  const [pageSize, setPageSize] = useState(50)
  const [onlyError, setOnlyError] = useState(false)
  const [onlyActionable, setOnlyActionable] = useState(false)
  const [autoRefresh, setAutoRefresh] = useState(true)
  const [selected, setSelected] = useState<string[]>([])
  const [selectAllTotal, setSelectAllTotal] = useState<number | null>(null)
  const [detail, setDetail] = useState<PostDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [batchAction, setBatchAction] = useState('approve')
  const [actionText, setActionText] = useState('')
  const [actionDelay, setActionDelay] = useState(180000)
  const [lastUpdatedAt, setLastUpdatedAt] = useState<number | null>(null)
  const [postView, setPostView] = useState<PostViewMode>('cards')
  const compactDetail = useMediaQuery('(max-width: 980px)')
  const canFilterError = allowOnlyError
  const canFilterActionable = allowOnlyActionable
  const canSelect = allowSelection
  const canBatch = allowBatchActions && allowSelection
  const canAct = allowActions
  const showDetailActions = allowDetailActions && allowActions

  const groups = useMemo(() => [...new Set(posts.map((post) => post.group_id))].sort(), [posts])
  const visiblePosts = useMemo(() => {
    let out = posts
    if (mode === 'sent') {
      out = out.filter((post) => post.stage === 'sent')
    } else if (stage === '__active__') {
      out = out.filter((post) => !ACTIVE_EXCLUDED.has(post.stage))
    } else if (stage) {
      out = out.filter((post) => post.stage === stage)
    }
    if (onlyError && canFilterError) out = out.filter((post) => !!post.last_error)
    if (onlyActionable && canFilterActionable) out = out.filter((post) => !!post.review_id)
    return out
  }, [posts, mode, stage, onlyError, onlyActionable, canFilterError, canFilterActionable])
  const selectableIds = useMemo(
    () => visiblePosts.map((post) => post.review_id).filter(Boolean) as string[],
    [visiblePosts],
  )
  const totalPages = Math.max(1, Math.ceil(total / pageSize))
  const currentSelectedCount = selectAllTotal ?? selected.length
  const detailIndex = detail ? visiblePosts.findIndex((post) => post.post_id === detail.post_id) : -1
  const showDetailPanel = !compactDetail && (!!detail || detailLoading)

  useEffect(() => {
    loadPosts()
  }, [stage, groupId, sortBy, sortOrder, page, pageSize, onlyError, onlyActionable])

  useEffect(() => {
    if (!autoRefresh) return
    const id = window.setInterval(() => loadPosts(), 30000)
    return () => window.clearInterval(id)
  }, [autoRefresh, stage, groupId, sortBy, sortOrder, page, pageSize, keyword, onlyError, onlyActionable])

  function currentQuery(overrides: Partial<PostQuerySnapshot> = {}): PostQuerySnapshot {
    return {
      stage,
      keyword,
      groupId,
      sortBy,
      sortOrder,
      page,
      pageSize,
      onlyError,
      onlyActionable,
      ...overrides,
    }
  }

  async function loadPosts({
    resetSelection = false,
    query = {},
  }: {
    resetSelection?: boolean
    query?: Partial<PostQuerySnapshot>
  } = {}) {
    setLoading(true)
    try {
      const params = buildPostParams(currentQuery(query))
      const result = await api<ListPostsResponse>('/api/posts?' + params.toString())
      setPosts(result.items)
      setTotal(result.total)
      setLastUpdatedAt(Date.now())
      if (resetSelection) {
        setSelected([])
        setSelectAllTotal(null)
      } else {
        const pageIds = new Set(result.items.map((post) => post.review_id).filter(Boolean) as string[])
        setSelected((prev) => prev.filter((id) => pageIds.has(id) || selectAllTotal !== null))
      }
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  function search() {
    setPage(0)
    setSelectAllTotal(null)
    setSelected([])
    void loadPosts({ resetSelection: true, query: { page: 0 } })
  }

  function resetFilters() {
    const nextQuery = {
      stage: mode === 'sent' ? 'sent' : initialStage,
      keyword: '',
      groupId: '',
      sortBy: 'created_at',
      sortOrder: 'desc' as SortOrder,
      page: 0,
      onlyError: false,
      onlyActionable: false,
    }
    setStage(initialStage)
    setKeyword('')
    setGroupId('')
    setSortBy('created_at')
    setSortOrder('desc')
    setOnlyError(false)
    setOnlyActionable(false)
    setPage(0)
    setSelected([])
    setSelectAllTotal(null)
    void loadPosts({ resetSelection: true, query: nextQuery })
  }

  function toggleOne(reviewId: string, checked: boolean) {
    setSelectAllTotal(null)
    setSelected((prev) => {
      if (checked) return prev.includes(reviewId) ? prev : [...prev, reviewId]
      return prev.filter((id) => id !== reviewId)
    })
  }

  function togglePageSelection() {
    setSelectAllTotal(null)
    setSelected((prev) => {
      const allSelected = selectableIds.length > 0 && selectableIds.every((id) => prev.includes(id))
      if (allSelected) return prev.filter((id) => !selectableIds.includes(id))
      return [...new Set([...prev, ...selectableIds])]
    })
  }

  async function selectAcrossPages() {
    setLoading(true)
    try {
      const params = buildPostParams(currentQuery({ page: 0 }))
      params.delete('cursor')
      params.delete('limit')
      const result = await api<ListReviewIdsResponse>('/api/reviews/ids?' + params.toString())
      setSelected(result.review_ids)
      setSelectAllTotal(result.total)
      notify(result.total ? 'success' : 'info', result.total ? `已选择 ${result.total} 条` : '没有可选择的稿件')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  async function openDetail(postId: string) {
    setDetailLoading(true)
    try {
      setDetail(await api<PostDetail>('/api/posts/' + postId))
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setDetailLoading(false)
    }
  }

  async function refreshDetail() {
    if (!detail) return
    await openDetail(detail.post_id)
  }

  async function runAction(reviewId: string, action: string, textOverride?: string) {
    if (!confirmDangerousAction(action)) return
    setActionLoading(true)
    try {
      await api(`/api/reviews/${reviewId}/decision`, {
        method: 'POST',
        body: JSON.stringify(buildActionPayload(action, textOverride ?? actionText, actionDelay)),
      })
      notify('success', `已执行：${ACTION_LABELS[action] ?? action}`)
      setActionText('')
      await loadPosts({ resetSelection: true })
      await refreshDetail()
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setActionLoading(false)
    }
  }

  async function runBatch() {
    if (!selected.length) return
    if (!confirmDangerousAction(batchAction, currentSelectedCount)) return
    setActionLoading(true)
    try {
      await api('/api/reviews/batch', {
        method: 'POST',
        body: JSON.stringify({
          review_ids: selected,
          ...buildActionPayload(batchAction, actionText, actionDelay),
        }),
      })
      notify('success', `批量执行完成：${ACTION_LABELS[batchAction] ?? batchAction}`)
      setSelected([])
      setSelectAllTotal(null)
      await loadPosts({ resetSelection: true })
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setActionLoading(false)
    }
  }

  return (
    <div className="workspace">
      <header className="page-head">
        <div>
          <h1>稿件审核</h1>
          <p>{lastUpdatedAt ? `刷新 ${formatDateTime(lastUpdatedAt)}` : '等待刷新'}</p>
        </div>
        <div className="head-actions">
          <div className="layout-note">
            <PanelRightOpen size={16} />
            双栏审核
          </div>
          <Switch isSelected={autoRefresh} onChange={setAutoRefresh} size="sm">
            自动刷新
          </Switch>
          <Button size="sm" variant="secondary" onClick={() => loadPosts()}>
            <RefreshCcw size={16} />
            刷新
          </Button>
        </div>
      </header>

      <section className="metrics" aria-label="审核指标">
        <Metric label="当前结果" value={visiblePosts.length} tone="neutral" icon={<Inbox size={18} />} />
        <Metric
          label="可操作"
          value={visiblePosts.filter((post) => !!post.review_id).length}
          tone="good"
          icon={<CheckCircle2 size={18} />}
        />
        <Metric
          label="异常"
          value={visiblePosts.filter((post) => !!post.last_error).length}
          tone="bad"
          icon={<AlertCircle size={18} />}
        />
        <Metric label="已选" value={currentSelectedCount} tone="warn" icon={<Check size={18} />} />
      </section>

      {(allowStageFilter || allowOnlyError || allowOnlyActionable) ? (
        <Card className="control-card">
          <Card.Content>
            <div className="toolbar-grid">
              {allowStageFilter ? (
                <HeroSelect
                  className="control"
                  ariaLabel="选择审核阶段"
                  selectedKey={stage}
                  options={stageOptions}
                  onSelect={(value) => {
                    setStage(value as Stage)
                    setPage(0)
                  }}
                />
              ) : null}
              <HeroSelect
                className="control"
                ariaLabel="选择稿件分组"
                selectedKey={groupId}
                options={[{ value: '', label: '' }, ...groups.map((group) => ({ value: group, label: group }))]}
                onSelect={(value) => {
                  setGroupId(value)
                  setPage(0)
                }}
              />
              <HeroSelect
                className="control"
                ariaLabel="选择排序"
                selectedKey={`${sortBy}:${sortOrder}`}
                options={SORT_OPTIONS}
                onSelect={(value) => {
                  const [nextSortBy, nextSortOrder] = value.split(':')
                  setSortBy(nextSortBy)
                  setSortOrder(nextSortOrder as SortOrder)
                  setPage(0)
                }}
              />
              <Input
                className="search-control"
                placeholder="搜索稿件编号、内容、用户"
                value={keyword}
                onChange={(event) => setKeyword(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') search()
                }}
              />
              <Button variant="secondary" onClick={search}>
                <Search size={16} />
                搜索
              </Button>
              <Button variant="tertiary" onClick={resetFilters}>
                重置
              </Button>
            </div>
            <div className="filter-row">
              {allowOnlyActionable ? <Checkbox isSelected={onlyActionable} onChange={setOnlyActionable}>仅可操作</Checkbox> : null}
              {allowOnlyError ? <Checkbox isSelected={onlyError} onChange={setOnlyError}>??</Checkbox> : null}
            </div>
          </Card.Content>
        </Card>
      ) : null}

      {canBatch ? (
        <Card className="control-card">
          <Card.Content>
            <div className="batch-row">
              <div className="batch-actions">
                <Button size="sm" variant="secondary" onClick={togglePageSelection}>
                  {selectableIds.length > 0 && selectableIds.every((id) => selected.includes(id))
                    ? ''
                    : ''}
                </Button>
                <Button size="sm" variant="secondary" onClick={selectAcrossPages}>
                选择全部
                </Button>
                {currentSelectedCount > 0 && (
                  <Button
                    size="sm"
                    variant="tertiary"
                    onClick={() => {
                      setSelected([])
                      setSelectAllTotal(null)
                    }}
                  >
                    清空选择
                  </Button>
                )}
              </div>
              <div className="batch-actions batch-actions-right">
                <HeroSelect
                  className="action-select"
                  ariaLabel="选择批量操作"
                  selectedKey={batchAction}
                  options={BATCH_ACTIONS.map((action) => ({ value: action, label: ACTION_LABELS[action] ?? action }))}
                  onSelect={setBatchAction}
                />
                {batchAction === 'defer' && (
                  <DelayField value={actionDelay} onChange={setActionDelay} className="delay-field" />
                )}
                <Button size="sm" isDisabled={!selected.length || actionLoading} onClick={runBatch}>
                  {actionLoading ? <Spinner size="sm" /> : <Check size={16} />}
                  批量执行
                </Button>
              </div>
            </div>
          </Card.Content>
        </Card>
      ) : null}

      <section className={showDetailPanel ? 'review-board has-detail' : 'review-board'}>
        <div className="review-feed">
          <section className="feed-panel">
            <header className="feed-head">
              <div>
                <h2>稿件队列</h2>
                <p>选择左侧稿件后在右侧处理详情</p>
              </div>
              <ToggleButtonGroup
                className="view-toggle"
                selectionMode="single"
                selectedKeys={[postView]}
                onSelectionChange={(keys) => {
                  const next = Array.from(keys)[0]
                  if (next) setPostView(String(next) as PostViewMode)
                }}
                size="sm"
                aria-label="稿件视图"
              >
                <ToggleButton id="cards">
                  <LayoutGrid size={16} />
                  卡片
                </ToggleButton>
                <ToggleButton id="list">
                  <List size={16} />
                  列表
                </ToggleButton>
              </ToggleButtonGroup>
            </header>
            <div className="feed-content">
              {loading && !posts.length ? (
                <EmptyPanel icon={<Spinner />} text="正在加载稿件" />
              ) : visiblePosts.length ? (
                postView === 'cards' ? (
                  <PostCards
                    posts={visiblePosts}
                    activePostId={detail?.post_id ?? null}
                    selected={selected}
                    selectAllTotal={selectAllTotal}
                    actionLoading={actionLoading}
                    allowSelection={canSelect}
                    allowActions={canAct}
                    onToggle={toggleOne}
                    onOpen={openDetail}
                    onAction={(reviewId, action, text) => runAction(reviewId, action, text)}
                  />
                ) : (
                  <PostTable
                    posts={visiblePosts}
                    activePostId={detail?.post_id ?? null}
                    selected={selected}
                    selectAllTotal={selectAllTotal}
                    actionLoading={actionLoading}
                    allowSelection={canSelect}
                    allowActions={canAct}
                    onToggle={toggleOne}
                    onOpen={openDetail}
                    onAction={(reviewId, action, text) => runAction(reviewId, action, text)}
                  />
                )
              ) : (
                <EmptyPanel icon={<Inbox size={28} />} text="没有符合条件的稿件" />
              )}
            </div>
          </section>

          <Pagination className="pager" size="sm" aria-label="稿件分页">
            <Pagination.Summary>
              共 {total} 条，第 {page + 1}/{totalPages} 页
            </Pagination.Summary>
            <HeroSelect
              className="page-size-select"
              ariaLabel="每页条数"
              selectedKey={String(pageSize)}
              options={PAGE_SIZES.map((size) => ({ value: String(size), label: `${size} 条/页` }))}
              onSelect={(value) => {
                setPageSize(Number(value))
                setPage(0)
              }}
            />
            <Pagination.Content>
              <Pagination.Item>
                <Pagination.Previous
                  isDisabled={page <= 0}
                  onPress={() => setPage((value) => Math.max(0, value - 1))}
                >
                  上一页
                </Pagination.Previous>
              </Pagination.Item>
              <Pagination.Item>
                <Pagination.Link isActive>{page + 1}</Pagination.Link>
              </Pagination.Item>
              <Pagination.Item>
                <Pagination.Next
                  isDisabled={page >= totalPages - 1}
                  onPress={() => setPage((value) => value + 1)}
                >
                  下一页
                </Pagination.Next>
              </Pagination.Item>
            </Pagination.Content>
          </Pagination>
        </div>

        {showDetailPanel && (
          <aside className="detail-column">
            <InlineDetailPanel
              detail={detail}
              loading={detailLoading}
              actionLoading={actionLoading}
              actionText={actionText}
              actionDelay={actionDelay}
              hasPrev={detailIndex > 0}
              hasNext={detailIndex >= 0 && detailIndex < visiblePosts.length - 1}
              allowActions={canAct}
              onClose={() => setDetail(null)}
              onRefresh={refreshDetail}
              onTextChange={setActionText}
              onDelayChange={setActionDelay}
              onAction={(action) => detail?.review_id && runAction(detail.review_id, action)}
              onPrev={() => detailIndex > 0 && openDetail(visiblePosts[detailIndex - 1].post_id)}
              onNext={() =>
                detailIndex >= 0 &&
                detailIndex < visiblePosts.length - 1 &&
                openDetail(visiblePosts[detailIndex + 1].post_id)
              }
            />
          </aside>
        )}
      </section>

      {compactDetail && (
        <DetailDrawer
          detail={detail}
          loading={detailLoading}
          actionLoading={actionLoading}
          actionText={actionText}
          actionDelay={actionDelay}
          hasPrev={detailIndex > 0}
          hasNext={detailIndex >= 0 && detailIndex < visiblePosts.length - 1}
          allowActions={canAct}
          onClose={() => setDetail(null)}
          onRefresh={refreshDetail}
          onTextChange={setActionText}
          onDelayChange={setActionDelay}
          onAction={(action) => detail?.review_id && runAction(detail.review_id, action)}
          onPrev={() => detailIndex > 0 && openDetail(visiblePosts[detailIndex - 1].post_id)}
          onNext={() =>
            detailIndex >= 0 &&
            detailIndex < visiblePosts.length - 1 &&
            openDetail(visiblePosts[detailIndex + 1].post_id)
          }
        />
      )}
    </div>
  )
}

function PostCards({
  posts,
  activePostId,
  selected,
  selectAllTotal,
  actionLoading,
  allowSelection,
  allowActions,
  onToggle,
  onOpen,
  onAction,
}: {
  posts: PostItem[]
  activePostId: string | null
  selected: string[]
  selectAllTotal: number | null
  actionLoading: boolean
  allowSelection: boolean
  allowActions: boolean
  onToggle: (reviewId: string, checked: boolean) => void
  onOpen: (postId: string) => void
  onAction: (reviewId: string, action: string, text?: string) => void
}) {
  const gridRef = useMasonryLayout([posts, activePostId, actionLoading])
  const [cardNotes, setCardNotes] = useState<Record<string, string>>({})

  function noteKey(post: PostItem) {
    return post.review_id ?? post.post_id
  }

  function updateNote(post: PostItem, value: string) {
    const key = noteKey(post)
    setCardNotes((prev) => ({ ...prev, [key]: value }))
  }

  function runCardAction(post: PostItem, action: string) {
    if (!post.review_id) return
    onAction(post.review_id, action, cardNotes[noteKey(post)] ?? '')
  }

  return (
    <div ref={gridRef} className="post-card-grid">
      {posts.map((post) => (
        <PostCard
          key={post.post_id}
          post={post}
          active={activePostId === post.post_id}
          selected={selected}
          selectAllTotal={selectAllTotal}
          actionLoading={actionLoading}
          allowSelection={allowSelection}
          allowActions={allowActions}
          note={cardNotes[noteKey(post)] ?? ''}
          onNoteChange={(value) => updateNote(post, value)}
          onToggle={onToggle}
          onOpen={onOpen}
          onAction={(action) => runCardAction(post, action)}
        />
      ))}
    </div>
  )
}

function PostCard({
  post,
  active,
  selected,
  selectAllTotal,
  actionLoading,
  allowSelection,
  allowActions,
  note,
  onNoteChange,
  onToggle,
  onOpen,
  onAction,
}: {
  post: PostItem
  active: boolean
  selected: string[]
  selectAllTotal: number | null
  actionLoading: boolean
  allowSelection: boolean
  allowActions: boolean
  note: string
  onNoteChange: (value: string) => void
  onToggle: (reviewId: string, checked: boolean) => void
  onOpen: (postId: string) => void
  onAction: (action: string) => void
}) {
  const imageUrls = post.preview_image_urls?.length ? post.preview_image_urls : post.preview_image_url ? [post.preview_image_url] : []
  const imageCount = post.preview_image_count ?? imageUrls.length

  return (
    <article className="post-card-wrap">
      <Card
        className={active ? 'post-card active' : 'post-card'}
        variant="secondary"
      >
        <Card.Header className="post-card-head">
          <button className="post-card-title-button" type="button" onClick={() => onOpen(post.post_id)}>
            <Card.Title>#{post.internal_code ?? post.external_code ?? '-'}</Card.Title>
            <Card.Description>{post.sender_id ?? '未知投稿人'}</Card.Description>
          </button>
          <div className="post-card-head-actions">
            <StageChip stage={post.stage} />
            {imageCount > 0 && (
              <Chip size="sm" variant="soft">
                {imageCount} 图
              </Chip>
            )}
            {allowSelection && post.review_id && (
              <Checkbox
                aria-label={`选择 ${post.internal_code ?? post.external_code ?? post.post_id}`}
                isSelected={selectAllTotal !== null || selected.includes(post.review_id)}
                onChange={(checked) => onToggle(post.review_id!, checked)}
              />
            )}
          </div>
        </Card.Header>
        <Card.Content className="post-card-content">
          <button className="post-card-body" type="button" onClick={() => onOpen(post.post_id)}>
            {post.preview_text && imageUrls.length === 0 && (
              <span className="post-card-preview">{post.preview_text}</span>
            )}
            {imageUrls.length > 0 ? (
              <DynamicPreviewImages urls={imageUrls} totalCount={imageCount} />
            ) : (
              !post.preview_text && (
                <span className="post-card-preview muted">
                  {post.last_error ? '该稿件存在异常信息' : '点击查看稿件详情'}
                </span>
              )
            )}
          </button>
          <div className="post-card-meta">
            <Chip size="sm" variant="soft">
              {post.group_id}
            </Chip>
            <span>{formatDateTime(post.created_at_ms)}</span>
          </div>
          {post.review_id && (
            <TextArea
              className="post-card-note"
              placeholder="评论或拒绝/拉黑原因"
              value={note}
              onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) => onNoteChange(event.target.value)}
            />
          )}
          {post.last_error && <div className="post-card-error">{post.last_error}</div>}
        </Card.Content>
        <Card.Footer className="post-card-footer">
          {allowActions && post.review_id ? (
            <div className="post-card-quick-actions">
              {CARD_QUICK_ACTIONS.map((action) => (
                <Button
                  key={action}
                  size="sm"
                  variant={quickActionVariant(action)}
                  className={`action-button action-${action}`}
                  isDisabled={actionLoading}
                  onClick={() => onAction(action)}
                >
                  {quickActionIcon(action)}
                  {cardActionLabel(action)}
                </Button>
              ))}
            </div>
          ) : (
            <span className="post-card-no-action">当前阶段暂无可执行动作</span>
          )}
        </Card.Footer>
      </Card>
    </article>
  )
}

function DynamicPreviewImages({ urls, totalCount }: { urls: string[]; totalCount: number }) {
  const shown = urls.slice(0, 6)
  if (shown.length === 1) {
    return <DynamicPreviewImage src={shown[0]} />
  }
  const cols = shown.length <= 2 ? shown.length : shown.length <= 4 ? 2 : 3
  const hiddenCount = Math.max(0, totalCount - shown.length)

  return (
    <span className="post-card-image-grid" style={{ '--image-cols': String(cols) } as CSSProperties}>
      {shown.map((src, index) => (
        <span className="post-card-grid-image-frame" key={`${src}-${index}`}>
          <img className="post-card-image" src={src} alt="稿件预览" loading="lazy" />
          {hiddenCount > 0 && index === shown.length - 1 && <span className="post-card-image-more">+{hiddenCount}</span>}
        </span>
      ))}
    </span>
  )
}

function DynamicPreviewImage({ src }: { src: string }) {
  const [ratio, setRatio] = useState(1)
  const safeRatio = Math.min(1.85, Math.max(0.72, ratio))
  const style = { '--preview-aspect': String(safeRatio) } as CSSProperties

  return (
    <span className="post-card-image-frame" style={style}>
      <img
        className="post-card-image"
        src={src}
        alt="稿件预览"
        loading="lazy"
        onLoad={(event) => {
          const image = event.currentTarget
          if (image.naturalWidth > 0 && image.naturalHeight > 0) {
            setRatio(image.naturalWidth / image.naturalHeight)
          }
        }}
      />
    </span>
  )
}

function PostTable({
  posts,
  activePostId,
  selected,
  selectAllTotal,
  actionLoading,
  allowSelection,
  allowActions,
  onToggle,
  onOpen,
  onAction,
}: {
  posts: PostItem[]
  activePostId: string | null
  selected: string[]
  selectAllTotal: number | null
  actionLoading: boolean
  allowSelection: boolean
  allowActions: boolean
  onToggle: (reviewId: string, checked: boolean) => void
  onOpen: (postId: string) => void
  onAction: (reviewId: string, action: string, text?: string) => void
}) {
  return (
    <div className="post-table" role="region" aria-label="稿件列表">
      <div className="post-table-scroll">
        <table>
          <thead>
            <tr>
              <th style={{ width: 72 }}>选择</th>
              <th style={{ width: 92 }}>编号</th>
              <th style={{ width: 102 }}>状态</th>
              <th>内容</th>
              <th style={{ width: 116 }}>时间</th>
              <th style={{ width: 264 }}>操作</th>
            </tr>
          </thead>
          <tbody>
            {posts.map((post) => (
              <tr
                key={post.post_id}
                className={activePostId === post.post_id ? 'current-row clickable-row' : 'clickable-row'}
                onClick={() => onOpen(post.post_id)}
              >
                <td>
                  <div className="list-check-slot" onClick={(event) => event.stopPropagation()}>
                    {allowSelection && post.review_id ? (
                      <button
                        type="button"
                        className={
                          selectAllTotal !== null || selected.includes(post.review_id)
                            ? 'list-checkbox checked'
                            : 'list-checkbox'
                        }
                        aria-label={`选择 ${post.internal_code ?? post.external_code ?? post.post_id}`}
                        aria-pressed={selectAllTotal !== null || selected.includes(post.review_id)}
                        onClick={() =>
                          onToggle(post.review_id!, !(selectAllTotal !== null || selected.includes(post.review_id!)))
                        }
                      >
                        <Check size={14} />
                      </button>
                    ) : (
                      <span className="list-check-placeholder" aria-hidden="true" />
                    )}
                  </div>
                </td>
                <td className="mono">#{post.internal_code ?? post.external_code ?? '-'}</td>
                <td>
                  <StageChip stage={post.stage} />
                </td>
                <td>
                  <div className="list-preview">
                    <div className="preview">{post.preview_text || (post.preview_image_url ? '[图片]' : '-')}</div>
                    <span>
                      {post.group_id} · {post.sender_id ?? '未知投稿人'}
                    </span>
                  </div>
                </td>
                <td>{formatDateTime(post.created_at_ms)}</td>
                <td>
                  {allowActions && post.review_id ? (
                    <div className="list-actions" onClick={(event) => event.stopPropagation()}>
                      {LIST_PRIMARY_ACTIONS.map((action) => (
                        <Button
                          key={action}
                          size="sm"
                          variant={quickActionVariant(action)}
                          className={`action-button action-${action}`}
                          isDisabled={actionLoading}
                          onClick={() => onAction(post.review_id!, action)}
                        >
                          {quickActionIcon(action)}
                          {cardActionLabel(action)}
                        </Button>
                      ))}
                      <ListMoreActions post={post} actionLoading={actionLoading} onAction={onAction} />
                    </div>
                  ) : (
                    <span className="post-card-no-action">不可操作</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

function ListMoreActions({
  post,
  actionLoading,
  onAction,
}: {
  post: PostItem
  actionLoading: boolean
  onAction: (reviewId: string, action: string, text?: string) => void
}) {
  function runMenuAction(action: string) {
    if (!post.review_id) return
    const text = promptListActionText(action)
    if (text === null) return
    onAction(post.review_id, action, text)
  }

  return (
    <Popover>
      <Popover.Trigger>
        <Button size="sm" variant="secondary" className="action-button action-more" isDisabled={actionLoading}>
          <MoreHorizontal size={16} />
          更多
        </Button>
      </Popover.Trigger>
      <Popover.Content placement="bottom end" className="list-more-popover" onClick={(event) => event.stopPropagation()}>
        <Popover.Dialog>
          <div className="list-more-menu">
            {CARD_QUICK_ACTIONS.map((action) => (
              <Button
                key={action}
                size="sm"
                variant={quickActionVariant(action)}
                className={`action-button action-${action}`}
                isDisabled={actionLoading}
                onClick={() => runMenuAction(action)}
              >
                {quickActionIcon(action)}
                {cardActionLabel(action)}
              </Button>
            ))}
          </div>
        </Popover.Dialog>
      </Popover.Content>
    </Popover>
  )
}

function promptListActionText(action: string) {
  if (action === 'comment') {
    const text = window.prompt('请输入评论内容')
    if (!text?.trim()) return null
    return text
  }
  if (action === 'blacklist') {
    return window.prompt('可填写拉黑原因，留空将直接拉黑') ?? null
  }
  return undefined
}

function confirmDangerousAction(action: string, count = 1) {
  if (!DANGEROUS_ACTIONS.has(action)) return true
  const label = ACTION_LABELS[action] ?? action
  const target = count > 1 ? `${count} 条稿件` : '当前稿件'
  return window.confirm(`确定要${label}${target}吗？此操作提交后会立即生效。`)
}

type DetailContentProps = {
  detail: PostDetail | null
  loading: boolean
  actionLoading: boolean
  actionText: string
  actionDelay: number
  hasPrev: boolean
  allowActions: boolean
  hasNext: boolean
  onRefresh: () => void
  onTextChange: (value: string) => void
  onDelayChange: (value: number) => void
  onAction: (action: string) => void
  onPrev: () => void
  onNext: () => void
}

function InlineDetailPanel(props: DetailContentProps & { onClose: () => void }) {
  const { detail, loading, onClose } = props

  return (
    <section className="inline-detail-panel">
      <header className="inline-detail-head">
        <div>
          <span className="eyebrow">稿件详情</span>
          <h2>{detail ? `#${detail.review_code ?? detail.external_code ?? '-'}` : '选择稿件'}</h2>
          <p>{detail ? '在右侧直接审核当前稿件' : '从左侧队列打开一条稿件'}</p>
        </div>
        {detail && (
          <Button size="sm" variant="tertiary" isIconOnly onPress={onClose} aria-label="关闭详情">
            <X size={16} />
          </Button>
        )}
      </header>
      <div className="inline-detail-body">
        {loading || detail ? (
          <DetailContent {...props} />
        ) : (
          <EmptyPanel icon={<PanelRightOpen size={28} />} text="左侧选择稿件后在这里审核" />
        )}
      </div>
    </section>
  )
}

function DetailContent({
  detail,
  loading,
  actionLoading,
  actionText,
  actionDelay,
  hasPrev,
  hasNext,
  allowActions,
  onRefresh,
  onTextChange,
  onDelayChange,
  onAction,
  onPrev,
  onNext,
}: DetailContentProps) {
  const [action, setAction] = useState('approve')

  useEffect(() => {
    onTextChange('')
  }, [action])

  const needsText = ['comment', 'reply', 'blacklist', 'quick_reply', 'merge'].includes(action)
  const textPlaceholder = action === 'merge' ? '目标审核编号' : action === 'quick_reply' ? '快捷回复键名' : '内容'

  if (loading || !detail) {
    return <EmptyPanel icon={<Spinner />} text="正在加载详情" />
  }

  return (
    <>
      <div className="drawer-tools">
        <Button size="sm" variant="secondary" isDisabled={!hasPrev} onClick={onPrev}>
          上一条
        </Button>
        <Button size="sm" variant="secondary" isDisabled={!hasNext} onClick={onNext}>
          下一条
        </Button>
        <Button size="sm" variant="secondary" onClick={onRefresh}>
          <RefreshCcw size={16} />
          刷新
        </Button>
      </div>

      <div className="detail-meta">
        <StageChip stage={detail.stage} />
        <Chip color={detail.is_safe ? 'success' : 'danger'} variant="soft" size="sm">
          {detail.is_safe ? '安全' : '待核查'}
        </Chip>
        <Chip color={detail.is_anonymous ? 'accent' : 'default'} variant="soft" size="sm">
          {detail.is_anonymous ? '匿名' : '非匿名'}
        </Chip>
      </div>

      <Card className="detail-card" variant="secondary">
        <Card.Content>
          <dl className="kv">
            <div>
              <dt>分组</dt>
              <dd>{detail.group_id}</dd>
            </div>
            <div>
              <dt>投稿人</dt>
              <dd className="mono">{detail.sender_id ?? '-'}</dd>
            </div>
            <div>
              <dt>时间</dt>
              <dd>{formatDateTime(detail.created_at_ms)}</dd>
            </div>
            <div>
              <dt>会话</dt>
              <dd className="mono">{detail.session_id}</dd>
            </div>
          </dl>
        </Card.Content>
      </Card>

      {allowActions ? (
        <Card className="action-card">
          <Card.Content>
            <div className="quick-action-panel">
              {DETAIL_QUICK_ACTIONS.map((item) => (
                <Button
                  key={item}
                  size="sm"
                  variant={quickActionVariant(item)}
                  className={`action-button action-${item}`}
                  isDisabled={!detail.review_id || actionLoading}
                  onClick={() => onAction(item)}
                >
                  {quickActionIcon(item)}
                  {ACTION_LABELS[item] ?? item}
                </Button>
              ))}
            </div>
            <div className="action-box">
              <HeroSelect
                className="action-select detail-action"
                ariaLabel="选择详情操作"
                selectedKey={action}
                options={DETAIL_ACTIONS.map((item) => ({ value: item, label: ACTION_LABELS[item] ?? item }))}
                onSelect={setAction}
              />
              {action === 'defer' && (
                <DelayField value={actionDelay} onChange={onDelayChange} className="delay-field" />
              )}
              {needsText &&
                (action === 'comment' || action === 'reply' || action === 'blacklist' ? (
                  <TextArea
                    className="action-text"
                    placeholder={textPlaceholder}
                    value={actionText}
                    onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) => onTextChange(event.target.value)}
                  />
                ) : (
                  <Input
                    className="action-text"
                    placeholder={textPlaceholder}
                    value={actionText}
                    onChange={(event) => onTextChange(event.target.value)}
                  />
                ))}
              <Button isDisabled={!detail.review_id || actionLoading} onClick={() => onAction(action)}>
                {actionLoading ? <Spinner size="sm" /> : <Send size={16} />}
                执行
              </Button>
            </div>
          </Card.Content>
        </Card>
      ) : null}
      {detail.render_png_blob_id && (
        <Card className="image-card" variant="secondary">
          <Card.Content>
            <img src={`/api/blobs/${detail.render_png_blob_id}`} alt="渲染预览" />
          </Card.Content>
        </Card>
      )}

      {detail.last_error && (
        <Card className="error-card" variant="secondary">
          <Card.Content>
            <pre>{detail.last_error}</pre>
          </Card.Content>
        </Card>
      )}
    </>
  )
}

function DetailDrawer({
  detail,
  loading,
  actionLoading,
  actionText,
  actionDelay,
  hasPrev,
  hasNext,
  allowActions,
  onClose,
  onRefresh,
  onTextChange,
  onDelayChange,
  onAction,
  onPrev,
  onNext,
}: DetailContentProps & { onClose: () => void }) {
  return (
    <Drawer
      isOpen={!!detail}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <Drawer.Backdrop variant="blur" isDismissable>
        <Drawer.Content placement="right" className="detail-drawer">
          <Drawer.Dialog aria-label="稿件详情">
            <Drawer.Header className="drawer-head">
              <div>
                <span className="eyebrow">稿件详情</span>
                <Drawer.Heading>#{detail?.review_code ?? detail?.external_code ?? '-'}</Drawer.Heading>
              </div>
              <Button size="sm" variant="tertiary" isIconOnly onPress={onClose} aria-label="关闭">
                <X size={16} />
              </Button>
            </Drawer.Header>
            <Drawer.Body className="drawer-body">
              <DetailContent
                detail={detail}
                loading={loading}
                actionLoading={actionLoading}
                actionText={actionText}
                actionDelay={actionDelay}
                hasPrev={hasPrev}
                hasNext={hasNext}
                allowActions={allowActions}
                onRefresh={onRefresh}
                onTextChange={onTextChange}
                onDelayChange={onDelayChange}
                onAction={onAction}
                onPrev={onPrev}
                onNext={onNext}
              />
            </Drawer.Body>
          </Drawer.Dialog>
        </Drawer.Content>
      </Drawer.Backdrop>
    </Drawer>
  )
}

function LegacySendSuccessReplySettingsView({
  notify,
}: {
  notify: (kind: ToastKind, text: string) => void
}) {
  /*
  const [settings, setSettings] = useState<SendSuccessReplySettingsResponse | null>(null)
  const [selectedGroup, setSelectedGroup] = useState('')
  const [enabled, setEnabled] = useState(true)
  const [textTemplate, setTextTemplate] = useState('#<code>已发送')
  const [images, setImages] = useState<string[]>([])
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const uploadInputRef = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    void loadSettings()
  }, [])

  function applySettings(result: SendSuccessReplySettingsResponse) {
    setSettings(result)
    setSelectedGroup(result.group_id)
    setEnabled(result.enabled)
    setTextTemplate(result.text_template)
    setImages(result.images)
  }

  async function loadSettings(groupId?: string) {
    setLoading(true)
    try {
      const params = new URLSearchParams()
      const targetGroup = (groupId ?? selectedGroup).trim()
      if (targetGroup) params.set('group_id', targetGroup)
      const suffix = params.toString() ? `?${params.toString()}` : ''
      const result = await api<SendSuccessReplySettingsResponse>(`/api/settings/send-success-reply${suffix}`)
      applySettings(result)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  async function saveSettings() {
    if (!selectedGroup) {
      notify('error', '请先选择要配置的群组')
      return
    }
    setSaving(true)
    try {
      const result = await api<SendSuccessReplySettingsResponse>('/api/settings/send-success-reply', {
        method: 'POST',
        body: JSON.stringify({
          group_id: selectedGroup,
          enabled,
          text_template: textTemplate,
          images,
        }),
      })
      applySettings(result)
      notify('success', '回执设置已保存，并已同步到运行时')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setSaving(false)
    }
  }

  function insertVariable(key: string) {
    const token = `<${key}>`
    setTextTemplate((prev) => `${prev}${token}`)
  }

  function updateImage(index: number, value: string) {
    setImages((prev) => prev.map((item, itemIndex) => (itemIndex === index ? value : item)))
  }

  function addImage() {
    setImages((prev) => [...prev, ''])
  }

  function removeImage(index: number) {
    setImages((prev) => prev.filter((_, itemIndex) => itemIndex !== index))
  }

  async function uploadImages(event: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? [])
    if (!files.length) return
    try {
      const uploaded = await Promise.all(files.map(readFileAsDataUrl))
      setImages((prev) => [...prev, ...uploaded])
      notify('success', `已添加 ${uploaded.length} 张图片`)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      event.target.value = ''
    }
  }

  if (loading && !settings) {
    return <EmptyPanel icon={<Spinner />} text="正在加载发稿回执设置" />
  }

  if (!settings) {
    return (
      <div className="workspace">
        <header className="page-head">
          <div>
            <h1>发稿回执</h1>
            <p>配置发送成功后返回给投稿用户的私聊文案和图片。</p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            重试
          </Button>
        </header>
        <Card className="panel-card">
          <Card.Content>
            <EmptyPanel icon={<Send size={28} />} text="当前没有可用的回执配置数据" />
          </Card.Content>
        </Card>
      </div>
    )
  }

  const groupOptions = settings.available_groups.map((group) => ({ value: group, label: group }))

  return (
    <div className="workspace settings-workspace">
      <header className="page-head">
        <div>
          <h1>发稿回执</h1>
          <p>保存后立即热更新，可配置文本模板、变量占位符和附带图片。</p>
        </div>
        <div className="head-actions">
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading || saving}
            onClick={() => void loadSettings(selectedGroup)}
          >
            <RefreshCcw size={16} />
            刷新
          </Button>
          <Button size="sm" isDisabled={loading || saving} onClick={() => void saveSettings()}>
            {saving ? <Spinner size="sm" /> : <Check size={16} />}
            保存
          </Button>
        </div>
      </header>

      <section className="settings-grid">
        <Card className="panel-card settings-main-card">
          <Card.Header>
            <Card.Title>文本模板</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-toolbar">
                <div className="field-stack">
                  <span className="field-label">群组</span>
                  <HeroSelect
                    className="control settings-group-select"
                    ariaLabel="选择群组"
                    selectedKey={selectedGroup}
                    options={groupOptions}
                    onSelect={(value) => {
                      setSelectedGroup(value)
                      void loadSettings(value)
                    }}
                  />
                </div>
                <div className="field-stack settings-switch-stack">
                  <span className="field-label">启用状态</span>
                  <Switch isSelected={enabled} onChange={setEnabled} size="sm">
                    {enabled ? '已启用' : '已关闭'}
                  </Switch>
                </div>
              </div>

              <div className="field-stack">
                <span className="field-label">回执文案</span>
                <TextArea
                  className="settings-template-textarea"
                  placeholder="#<code>已发送"
                  value={textTemplate}
                  onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                    setTextTemplate(event.target.value)
                  }
                />
                <span className="field-hint">
                  支持使用 <code>{'<变量>'}</code> 占位，文本会先发送，图片会按顺序附加。
                </span>
              </div>

              <div className="field-stack">
                <div className="settings-section-head">
                  <span className="field-label">可插入变量</span>
                  <span className="field-hint">点击即可追加到文案末尾，图片地址同样支持变量。</span>
                </div>
                <div className="settings-variable-grid">
                  {settings.variables.map((variable) => (
                    <button
                      key={variable.key}
                      type="button"
                      className="settings-variable-button"
                      onClick={() => insertVariable(variable.key)}
                    >
                      <strong>{variable.label}</strong>
                      <code>{variable.example}</code>
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card settings-side-card">
          <Card.Header>
            <Card.Title>附带图片</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-image-actions">
                <Button size="sm" variant="secondary" onClick={addImage}>
                  新增一项
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => uploadInputRef.current?.click()}
                >
                  上传图片
                </Button>
                <input
                  ref={uploadInputRef}
                  className="sr-only"
                  type="file"
                  accept="image/*"
                  multiple
                  onChange={uploadImages}
                />
              </div>
              <span className="field-hint">
                可填写远程图片 URL，也可直接上传，上传后会保存为 data URL。
              </span>
              {images.length ? (
                <div className="settings-image-list">
                  {images.map((image, index) => {
                    const previewable = isPreviewableImageSource(image)
                    return (
                      <div key={`reply-image-${index}`} className="settings-image-item">
                        <div className="settings-image-row">
                          <Input
                            className="settings-image-input"
                            placeholder="https://example.com/success.png 或 data:image/..."
                            value={image}
                            onChange={(event) => updateImage(index, event.target.value)}
                          />
                          <Button
                            size="sm"
                            variant="tertiary"
                            isIconOnly
                            aria-label={`删除图片 ${index + 1}`}
                            onClick={() => removeImage(index)}
                          >
                            <Trash2 size={16} />
                          </Button>
                        </div>
                        <div className="settings-image-meta">
                          <span className="field-hint">{buildReplyImageLabel(image, index)}</span>
                        </div>
                        {previewable && (
                          <div className="settings-image-preview-frame">
                            <img
                              className="settings-image-preview"
                              src={image.trim()}
                              alt={`回执图片预览 ${index + 1}`}
                            />
                          </div>
                        )}
                        </div>
                        <div
                          className={`agent-block-dropzone ${
                            dragState?.commandIndex === commandIndex &&
                            dragState?.blockIndex === blockIndex &&
                            dragState?.position === 'after'
                              ? 'is-active'
                              : ''
                          }`}
                          onDragOver={(event) => event.preventDefault()}
                          onDragEnter={handleDropZoneEnter(commandIndex, blockIndex, 'after')}
                          onDrop={performDrop(commandIndex, blockIndex, 'after')}
                        />
                      </div>
                    )
                  })}
                </div>
              ) : (
                <div className="settings-empty">
                  <MessageSquare size={20} />
                  <span>当前未附带图片，保存后将只发送文案。</span>
                </div>
              )}
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card settings-side-card">
          <Card.Header>
            <Card.Title>变量说明</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-variable-list">
              {settings.variables.map((variable) => (
                <div key={variable.key} className="settings-variable-card">
                  <div className="settings-variable-meta">
                    <strong>{variable.label}</strong>
                    <code>{variable.example}</code>
                  </div>
                  <p>{variable.description}</p>
                </div>
              ))}
            </div>
          </Card.Content>
        </Card>
      </section>
    </div>
  )
  */
  return <NotificationSettingsWorkbench notify={notify} />
}

function SettingsView({
  me,
  notify,
}: {
  me: MeResponse
  notify: (kind: ToastKind, text: string) => void
}) {
  const [tab, setTab] = useState<SettingsTabKey>(
    me.role === 'global_admin' ? 'config' : 'notifications'
  )
  /*
  const [tab, setTab] = useState<SettingsTabKey>(
    me.role === 'global_admin' ? 'config' : 'notifications'
  )

  if (me.role !== 'global_admin') {
    return <NotificationSettingsWorkbench notify={notify} />
  }

  const options = [
    {
      value: 'config' as const,
      label: '运行配置',
      description: '纯 GUI 编辑 config.json 的常用运行参数',
      icon: <LayoutGrid size={16} />,
    },
    {
      value: 'notifications' as const,
      label: '用户回链',
      description: '配置入队、发稿成功、拒稿时发给用户的消息',
      icon: <Send size={16} />,
    },
  ]

  return (
    <div className="workspace settings-hub">
      <Card className="panel-card">
        <Card.Content>
          <div className="settings-stage-grid settings-tab-grid">
            {options.map((option) => (
              <button
                key={option.value}
                type="button"
                className={`settings-stage-button ${tab === option.value ? 'is-active' : ''}`}
                onClick={() => setTab(option.value)}
              >
                <span className="settings-stage-icon">{option.icon}</span>
                <strong>{option.label}</strong>
                <span>{option.description}</span>
              </button>
            ))}
          </div>
        </Card.Content>
      </Card>
      {tab === 'config' ? (
        <RuntimeConfigWorkbench notify={notify} />
      ) : (
        <NotificationSettingsWorkbench notify={notify} />
      )}
    </div>
  )
  */
  if (me.role !== 'global_admin') {
    return <NotificationSettingsWorkbench notify={notify} />
  }

  const options = [
    {
      value: 'config' as const,
      label: 'Runtime Config',
      description: 'Edit config.json and runtime settings in the GUI.',
      icon: <LayoutGrid size={16} />,
    },
    {
      value: 'notifications' as const,
      label: 'User Replies',
      description: 'Configure queue, success, and rejection messages.',
      icon: <Send size={16} />,
    },
  ]

  return (
    <div className="workspace settings-hub">
      <SettingsMenuCard title="Settings" options={options} activeKey={tab} onSelect={setTab} />
      {tab === 'config' ? (
        <RuntimeConfigWorkbench notify={notify} />
      ) : (
        <NotificationSettingsWorkbench notify={notify} />
      )}
    </div>
  )
}

function TagMappingView({ notify }: { notify: (kind: ToastKind, text: string) => void }) {
  const [settings, setSettings] = useState<UserNotificationSettingsResponse | null>(null)
  const [selectedGroup, setSelectedGroup] = useState('')
  const [selectedTag, setSelectedTag] = useState('')
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)

  function normalizeVariableName(value: string) {
    return String(value ?? '')
      .trim()
      .replace(/^<+/, '')
      .replace(/>+$/, '')
      .trim()
  }

  function formatVariableName(value: string) {
    const normalized = normalizeVariableName(value)
    return normalized ? `<${normalized}>` : ''
  }

  useEffect(() => {
    void loadSettings()
  }, [])

  async function loadSettings(groupId?: string) {
    setLoading(true)
    try {
      const params = new URLSearchParams()
      const targetGroup = (groupId ?? selectedGroup).trim()
      if (targetGroup) params.set('group_id', targetGroup)
      const suffix = params.toString() ? `?${params.toString()}` : ''
      const result = await api<UserNotificationSettingsResponse>(
        `/api/settings/user-notifications${suffix}`
      )
      setSettings(result)
      setSelectedGroup(result.group_id)
      setSelectedTag((current) =>
        normalizeVariableName(current || result.tag_value_maps[0]?.tag || '')
      )
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  async function saveSettings() {
    if (!settings || !selectedGroup) {
      notify('error', '请先选择要配置的群组')
      return
    }
    setSaving(true)
    try {
      const result = await api<UserNotificationSettingsResponse>(
        '/api/settings/user-notifications',
        {
          method: 'POST',
          body: JSON.stringify({
            group_id: selectedGroup,
            queue_entered: settings.queue_entered,
            review_queued: settings.review_queued,
            send_succeeded: settings.send_succeeded,
            rejected: settings.rejected,
            webhook_tag_map: settings.webhook_tag_map,
            tag_value_maps: settings.tag_value_maps,
          }),
        }
      )
      setSettings(result)
      setSelectedGroup(result.group_id)
      setSelectedTag((current) =>
        normalizeVariableName(current || result.tag_value_maps[0]?.tag || '')
      )
      notify('success', '标签映射已保存')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setSaving(false)
    }
  }

  function updateTagGroups(
    updater: (groups: TagValueMappingGroup[]) => TagValueMappingGroup[]
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return { ...prev, tag_value_maps: updater(prev.tag_value_maps) }
    })
  }

  function normalizeTagMap(groups: TagValueMappingGroup[]) {
    return groups.map((group) => ({
      tag: normalizeVariableName(group.tag),
      mappings: group.mappings
        .map((entry) => ({
          source: String(entry.source ?? '').trim(),
          target: String(entry.target ?? '').trim(),
        }))
        .filter((entry) => entry.source && entry.target),
    }))
  }

  function currentTagGroup() {
    if (!settings) return null
    const selected = normalizeVariableName(selectedTag)
    return (
      settings.tag_value_maps.find((group) => normalizeVariableName(group.tag) === selected) ??
      null
    )
  }

  function ensureSelectedTag(groups: TagValueMappingGroup[]) {
    const current = normalizeVariableName(selectedTag)
    if (current && groups.some((group) => normalizeVariableName(group.tag) === current)) return current
    return normalizeVariableName(groups[0]?.tag ?? '')
  }

  function addTagGroup() {
    updateTagGroups((groups) => {
      const next = [...groups, { tag: '', mappings: [] }]
      setSelectedTag(ensureSelectedTag(next))
      return next
    })
  }

  function updateTagGroup(index: number, value: string) {
    const normalizedValue = normalizeVariableName(value)
    updateTagGroups((groups) =>
      groups.map((group, groupIndex) =>
        groupIndex === index ? { ...group, tag: normalizedValue } : group
      )
    )
    setSelectedTag(normalizedValue)
  }

  function removeTagGroup(index: number) {
    updateTagGroups((groups) => {
      const next = groups.filter((_, groupIndex) => groupIndex !== index)
      setSelectedTag(ensureSelectedTag(next))
      return next
    })
  }

  function updateMappingEntry(index: number, key: keyof TagValueMappingEntry, value: string) {
    const selected = normalizeVariableName(selectedTag)
    updateTagGroups((groups) =>
      groups.map((group) => {
        if (normalizeVariableName(group.tag) !== selected) return group
        return {
          ...group,
          mappings: group.mappings.map((entry, entryIndex) =>
            entryIndex === index ? { ...entry, [key]: value } : entry
          ),
        }
      })
    )
  }

  function addMappingEntry() {
    const selected = normalizeVariableName(selectedTag)
    updateTagGroups((groups) =>
      groups.map((group) => {
        if (normalizeVariableName(group.tag) !== selected) return group
        return { ...group, mappings: [...group.mappings, { source: '', target: '' }] }
      })
    )
  }

  function removeMappingEntry(index: number) {
    const selected = normalizeVariableName(selectedTag)
    updateTagGroups((groups) =>
      groups.map((group) => {
        if (normalizeVariableName(group.tag) !== selected) return group
        return {
          ...group,
          mappings: group.mappings.filter((_, sourceIndex) => sourceIndex !== index),
        }
      })
    )
  }

  if (loading && !settings) {
    return <EmptyPanel icon={<Spinner />} text="正在加载标签映射" />
  }

  if (!settings) {
    return (
      <div className="workspace">
        <header className="page-head">
          <div>
            <h1>标签映射</h1>
            <p>按变量名分别维护源字符串到实际发送值的映射列表。</p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            重试
          </Button>
        </header>
        <Card className="panel-card">
          <Card.Content>
            <EmptyPanel icon={<LayoutGrid size={28} />} text="当前没有可用的标签映射数据" />
          </Card.Content>
        </Card>
      </div>
    )
  }

  const tagOptions = settings.tag_value_maps.map((group) => ({ value: group.tag, label: group.tag }))
  const activeGroup = currentTagGroup() ?? settings.tag_value_maps[0] ?? null
  const mappingGroups = normalizeTagMap(settings.tag_value_maps)

  return (
    <div className="workspace settings-workspace">
      <header className="page-head">
        <div>
          <h1>标签映射</h1>
          <p>
            每个变量名单独维护一个映射列表，变量名会以 <code>&lt;stage&gt;</code>{' '}
            这种形式显示。
          </p>
        </div>
        <div className="head-actions">
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading || saving}
            onClick={() => void loadSettings(selectedGroup)}
          >
            <RefreshCcw size={16} />
            刷新
          </Button>
          <Button size="sm" isDisabled={loading || saving} onClick={() => void saveSettings()}>
            {saving ? <Spinner size="sm" /> : <Check size={16} />}
            保存
          </Button>
        </div>
      </header>

      <section className="settings-grid">
        <Card className="panel-card settings-main-card">
          <Card.Header>
            <Card.Title>变量分组</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-toolbar">
                <div className="field-stack">
                  <span className="field-label">运行分组</span>
                  <HeroSelect
                    className="control settings-group-select"
                    ariaLabel="选择运行分组"
                    selectedKey={selectedGroup}
                    options={settings.available_groups.map((group) => ({ value: group, label: group }))}
                    onSelect={(value) => {
                      setSelectedGroup(value)
                      void loadSettings(value)
                    }}
                  />
                </div>
              </div>

              <div className="settings-stage-grid">
                {mappingGroups.map((group, index) => (
                  <button
                    key={`tag-group-${index}-${group.tag}`}
                    type="button"
                    className={`settings-stage-button ${String(selectedTag) === String(group.tag) ? 'is-active' : ''}`}
                    onClick={() => setSelectedTag(normalizeVariableName(group.tag))}
                  >
                    <span className="settings-stage-icon"><LayoutGrid size={16} /></span>
                    <strong>{formatVariableName(group.tag) || '未命名变量'}</strong>
                    <span>{group.mappings.length} 条映射</span>
                  </button>
                ))}
              </div>

              <div className="settings-inline-grid">
                <div className="field-stack settings-switch-stack">
                  <span className="field-label">当前变量名</span>
                  <span className="field-hint">{formatVariableName(activeGroup?.tag ?? '') || '未选择'}</span>
                </div>
                <div className="field-stack settings-switch-stack">
                  <span className="field-label">管理变量</span>
                  <Button size="sm" variant="secondary" onClick={addTagGroup}>
                    新增变量
                  </Button>
                </div>
              </div>
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card settings-side-card">
          <Card.Header>
            <Card.Title>映射列表</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-section-head">
                <span className="field-hint">源字符串和变量名在匹配时都会先转换成字符串。</span>
                <Button size="sm" variant="secondary" onClick={addMappingEntry}>
                  新增映射
                </Button>
              </div>
              {activeGroup ? (
                <div className="settings-variable-list">
                  <div className="settings-variable-card">
                    <div className="settings-image-row">
                      <Input
                        className="settings-image-input"
                        placeholder="变量名，例如 &lt;stage&gt;"
                        value={formatVariableName(activeGroup.tag)}
                        onChange={(event) =>
                          updateTagGroup(
                            mappingGroups.findIndex((group) => String(group.tag) === String(activeGroup.tag)),
                            event.target.value
                          )
                        }
                      />
                    </div>
                  </div>
                  {activeGroup.mappings.length ? (
                    activeGroup.mappings.map((entry, index) => (
                      <div key={`tag-source-${selectedTag}-${index}`} className="settings-variable-card">
                        <div className="settings-image-row">
                          <Input
                            className="settings-image-input"
                            placeholder="源字符串"
                            value={entry.source}
                            onChange={(event) => updateMappingEntry(index, 'source', event.target.value)}
                          />
                          <Input
                            className="settings-image-input"
                            placeholder="目标字符串"
                            value={entry.target}
                            onChange={(event) => updateMappingEntry(index, 'target', event.target.value)}
                          />
                          <Button
                            size="sm"
                            variant="tertiary"
                            isIconOnly
                            aria-label={`删除映射 ${index + 1}`}
                            onClick={() => removeMappingEntry(index)}
                          >
                            <Trash2 size={16} />
                          </Button>
                        </div>
                      </div>
                    ))
                  ) : (
                    <div className="settings-empty">
                      <MessageSquare size={20} />
                      <span>当前变量还没有映射项。</span>
                    </div>
                  )}
                </div>
              ) : (
                <div className="settings-empty">
                  <MessageSquare size={20} />
                  <span>先新增一个变量，再维护它的映射列表。</span>
                </div>
              )}
            </div>
          </Card.Content>
        </Card>
      </section>
    </div>
  )
}

function AgentView({
  notify,
}: {
  notify: (kind: ToastKind, text: string) => void
}) {
  return <RuntimeConfigWorkbench notify={notify} mode="agent" />
}

function AgentVariableFragmentPalette({
  variables,
  onPick,
  className = '',
}: {
  variables: Array<AppConfigSettingsResponse['agent_command_variables'][number] & { token?: string }>
  onPick: (key: string) => void
  className?: string
}) {
  return (
    <div className={`agent-variable-fragment-grid ${className}`.trim()}>
      {variables.map((variable) => (
        <button
          key={variable.key}
          type="button"
          draggable
          className="agent-variable-fragment"
          onDragStart={(event) => {
            const payload: AgentVariableDragPayload = { source: 'variable', key: variable.key }
            event.dataTransfer.effectAllowed = 'copy'
            event.dataTransfer.setData(AGENT_BLOCK_DRAG_MIME, JSON.stringify(payload))
          }}
          onDragEnd={(event) => {
            event.dataTransfer.clearData()
          }}
          onClick={() => onPick(variable.key)}
        >
          <strong>{variable.label}</strong>
          <code>{variable.token ?? `<${variable.key}>`}</code>
          <span>{variable.description}</span>
        </button>
      ))}
    </div>
  )
}

function buildRuntimeConfigPages(): Array<{
  value: RuntimeConfigPageKey
  label: string
  description: string
  icon: ReactNode
}> {
  return [
    {
      value: 'common_runtime',
      label: '基础运行',
      description: '处理间隔、超时、缓存和时区。',
      icon: <Clock3 size={16} />,
    },
    {
      value: 'common_services',
      label: '服务端口',
      description: 'Web API、WebUI 和管理员入口。',
      icon: <LayoutGrid size={16} />,
    },
    {
      value: 'common_telemetry',
      label: '遥测',
      description: '遥测目录、上传和批量参数。',
      icon: <BarChart3 size={16} />,
    },
    {
      value: 'global_admins',
      label: '全局管理员',
      description: 'WebUI 全局管理员账号。',
      icon: <ShieldCheck size={16} />,
    },
    {
      value: 'group_basic',
      label: '分组基础',
      description: '当前分组基础信息和 NapCat 接入。',
      icon: <Inbox size={16} />,
    },
    {
      value: 'group_delivery',
      label: '分组发稿',
      description: '账号池、定时发稿、队列和图片限制。',
      icon: <Send size={16} />,
    },
    {
      value: 'group_quick_replies',
      label: '快捷回复',
      description: '管理用户快捷回复映射。',
      icon: <MessageSquare size={16} />,
    },
    {
      value: 'group_shortcuts',
      label: '快捷指令',
      description: '审核快捷指令和全局快捷指令。',
      icon: <Zap size={16} />,
    },
    {
      value: 'group_agent_commands',
      label: 'Agent 指令',
      description: '独立 Agent 页面入口。',
      icon: <FileText size={16} />,
    },
    {
      value: 'group_admins',
      label: '分组管理员',
      description: '当前分组 WebUI 管理员。',
      icon: <UserRound size={16} />,
    },
  ]
}

function SettingsMenuCard<T extends string>({
  title,
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
      <Card.Header>
        <Card.Title>{title}</Card.Title>
      </Card.Header>
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

function NotificationStageCardPreview({ images }: { images: string[] }) {
  const previewable = images
    .map((value) => value.trim())
    .filter((value) => isPreviewableImageSource(value))
    .slice(0, 3)

  if (!previewable.length) {
    return (
      <div className="stage-card-media stage-card-media-empty">
        <span>未附图</span>
      </div>
    )
  }

  return (
    <div className="stage-card-media">
      {previewable.map((image, index) => (
        <span key={`${image}-${index}`} className="stage-card-media-item">
          <img src={image} alt={`?? ${index + 1}`} loading="lazy" />
        </span>
      ))}
      {images.length > previewable.length ? (
        <span className="stage-card-media-more">+{images.length - previewable.length}</span>
      ) : null}
    </div>
  )
}

function StageTagManager({
  title,
  tags,
  onAdd,
  onChange,
  onRemove,
}: {
  title: string
  tags: string[]
  onAdd: () => void
  onChange: (index: number, value: string) => void
  onRemove: (index: number) => void
}) {
  return (
    <Card className="panel-card">
      <Card.Header>
        <Card.Title>{title}</Card.Title>
      </Card.Header>
      <Card.Content>
        <div className="settings-panel">
          <div className="settings-section-head">
            <span className="field-label">标签列表</span>
            <Button size="sm" variant="secondary" onClick={onAdd}>
              新增标签
            </Button>
          </div>
          {tags.length ? (
            <div className="settings-variable-list">
              {tags.map((tag, index) => (
                <div key={`${title}-${index}`} className="settings-variable-card">
                  <div className="settings-image-row">
                    <Input
                      className="settings-image-input"
                      placeholder="例如 <source_webhook_tag> 或 3344846508"
                      value={tag}
                      onChange={(event) => onChange(index, event.target.value)}
                    />
                    <Button
                      size="sm"
                      variant="tertiary"
                      isIconOnly
                      aria-label={`删除标签 ${index + 1}`}
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
              <span>当前阶段还没有配置标签。</span>
            </div>
          )}
        </div>
      </Card.Content>
    </Card>
  )
}

function AppConfigSettingsView({
  notify,
}: {
  notify: (kind: ToastKind, text: string) => void
}) {
  /*
  const [settings, setSettings] = useState<AppConfigSettingsResponse | null>(null)
  const [selectedGroup, setSelectedGroup] = useState('')
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    void loadSettings()
  }, [])

  function applySettings(result: AppConfigSettingsResponse) {
    setSettings(result)
    setSelectedGroup((prev) =>
      result.groups.some((group) => group.group_id === prev)
        ? prev
        : (result.groups[0]?.group_id ?? '')
    )
  }

  async function loadSettings() {
    setLoading(true)
    try {
      applySettings(await api<AppConfigSettingsResponse>('/api/settings/config'))
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  async function saveSettings() {
    if (!settings) return
    setSaving(true)
    try {
      const result = await api<AppConfigSettingsResponse>('/api/settings/config', {
        method: 'POST',
        body: JSON.stringify({
          common: settings.common,
          global_admins: settings.global_admins,
          groups: settings.groups,
        }),
      })
      applySettings(result)
      notify('success', '运行配置已保存到 config.json，部分运行参数需要重启 OQQWall 后生效')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setSaving(false)
    }
  }

  function updateCommon<K extends keyof AppConfigCommonSettings>(
    key: K,
    value: AppConfigCommonSettings[K]
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        common: {
          ...prev.common,
          [key]: value,
        },
      }
    })
  }

  function updateSelectedGroup(
    updater: (group: AppConfigGroupSettings) => AppConfigGroupSettings
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        groups: prev.groups.map((group) =>
          group.group_id === selectedGroup ? updater(group) : group
        ),
      }
    })
  }

  function updateGroupField<K extends keyof AppConfigGroupSettings>(
    key: K,
    value: AppConfigGroupSettings[K]
  ) {
    if (key === 'group_id') {
      setSelectedGroup(String(value))
    }
    updateSelectedGroup((group) => ({
      ...group,
      [key]: value,
    }))
  }

  function updateGroupStringList(field: 'accounts' | 'send_schedule', index: number, value: string) {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: group[field].map((item, itemIndex) => (itemIndex === index ? value : item)),
    }))
  }

  function addGroupStringListItem(field: 'accounts' | 'send_schedule') {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: [...group[field], ''],
    }))
  }

  function removeGroupStringListItem(field: 'accounts' | 'send_schedule', index: number) {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: group[field].filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function updateGroupMapping(
    field: 'quick_replies' | 'review_shortcuts' | 'global_shortcuts',
    index: number,
    key: keyof MappingEntry,
    value: string
  ) {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: group[field].map((item, itemIndex) =>
        itemIndex === index ? { ...item, [key]: value } : item
      ),
    }))
  }

  function addGroupMapping(field: 'quick_replies' | 'review_shortcuts' | 'global_shortcuts') {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: [...group[field], { key: '', value: '' }],
    }))
  }

  function removeGroupMapping(
    field: 'quick_replies' | 'review_shortcuts' | 'global_shortcuts',
    index: number
  ) {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: group[field].filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function updateGlobalAdmin(index: number, key: keyof ConfigAdminEntry, value: string | boolean) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        global_admins: prev.global_admins.map((item, itemIndex) =>
          itemIndex === index ? { ...item, [key]: value } : item
        ),
      }
    })
  }

  function addGlobalAdmin() {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        global_admins: [
          ...prev.global_admins,
          { id: '', username: '', password: '', password_set: false },
        ],
      }
    })
  }

  function removeGlobalAdmin(index: number) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        global_admins: prev.global_admins.filter((_, itemIndex) => itemIndex !== index),
      }
    })
  }

  function updateGroupAdmin(index: number, key: keyof ConfigAdminEntry, value: string | boolean) {
    updateSelectedGroup((group) => ({
      ...group,
      webview_admins: group.webview_admins.map((item, itemIndex) =>
        itemIndex === index ? { ...item, [key]: value } : item
      ),
    }))
  }

  function addGroupAdmin() {
    updateSelectedGroup((group) => ({
      ...group,
      webview_admins: [
        ...group.webview_admins,
        { id: '', username: '', password: '', password_set: false },
      ],
    }))
  }

  function removeGroupAdmin(index: number) {
    updateSelectedGroup((group) => ({
      ...group,
      webview_admins: group.webview_admins.filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function addGroup() {
    let nextGroupId = ''
    setSettings((prev) => {
      if (!prev) return prev
      nextGroupId = buildNextGroupId(prev.groups)
      return {
        ...prev,
        groups: [...prev.groups, buildDefaultConfigGroup(nextGroupId, prev.common)],
      }
    })
    if (nextGroupId) setSelectedGroup(nextGroupId)
  }

  function removeCurrentGroup() {
    if (!settings) return
    if (settings.groups.length <= 1) {
      notify('error', '至少保留一个分组配置')
      return
    }
    const activeGroup = settings.groups.find((group) => group.group_id === selectedGroup)
    if (!activeGroup) return
    if (!window.confirm(`确定删除分组 ${activeGroup.group_id} 吗？这会在保存后写回 config.json。`)) {
      return
    }
    const remainingGroups = settings.groups.filter((group) => group.group_id !== activeGroup.group_id)
    setSettings((prev) => (prev ? { ...prev, groups: remainingGroups } : prev))
    setSelectedGroup(remainingGroups[0]?.group_id ?? '')
  }

  if (loading && !settings) {
    return <EmptyPanel icon={<Spinner />} text="正在加载运行配置" />
  }

  if (!settings) {
    return (
      <div className="workspace">
        <header className="page-head">
          <div>
            <h1>运行配置</h1>
            <p>在 WebUI 中以纯 GUI 方式维护 config.json 的常用运行参数。</p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            重试
          </Button>
        </header>
        <Card className="panel-card">
          <Card.Content>
            <EmptyPanel icon={<LayoutGrid size={28} />} text="当前没有可用的运行配置数据" />
          </Card.Content>
        </Card>
      </div>
    )
  }

  const groups = settings.groups
  const activeGroup = groups.find((group) => group.group_id === selectedGroup) ?? groups[0] ?? null

  return (
    <div className="workspace settings-workspace">
      <header className="page-head">
        <div>
          <h1>运行配置</h1>
          <p>
            当前配置文件：<code>{settings.config_path}</code>。这里保存的是结构化 GUI 配置，不需要手写
            JSON；但大多数运行参数仍需重启 OQQWall 后生效。
          </p>
        </div>
        <div className="head-actions">
          <Button size="sm" variant="secondary" isDisabled={loading || saving} onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            刷新
          </Button>
          <Button size="sm" isDisabled={loading || saving} onClick={() => void saveSettings()}>
            {saving ? <Spinner size="sm" /> : <Check size={16} />}
            保存
          </Button>
        </div>
      </header>

      <section className="config-stack">
        <Card className="panel-card">
          <Card.Header>
            <Card.Title>基础配置</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="config-form-grid">
                <ConfigNumberField
                  label="处理等待（秒）"
                  value={settings.common.process_waittime_sec}
                  min={0}
                  onChange={(value) => updateCommon('process_waittime_sec', value)}
                />
                <ConfigNumberField
                  label="最小发送间隔（毫秒）"
                  value={settings.common.min_interval_ms}
                  min={0}
                  onChange={(value) => updateCommon('min_interval_ms', value)}
                />
                <ConfigNumberField
                  label="公共单条最大图片数"
                  value={settings.common.max_image_number_one_post}
                  min={1}
                  onChange={(value) => updateCommon('max_image_number_one_post', value)}
                />
                <ConfigNumberField
                  label="发送超时（毫秒）"
                  value={settings.common.send_timeout_ms}
                  min={0}
                  onChange={(value) => updateCommon('send_timeout_ms', value)}
                />
                <ConfigNumberField
                  label="发送重试次数"
                  value={settings.common.send_max_attempts}
                  min={0}
                  onChange={(value) => updateCommon('send_max_attempts', value)}
                />
                <ConfigNumberField
                  label="时区偏移（分钟）"
                  value={settings.common.tz_offset_minutes}
                  allowNegative
                  onChange={(value) => updateCommon('tz_offset_minutes', value)}
                />
                <ConfigNumberField
                  label="内存缓存（MB）"
                  value={settings.common.max_cache_mb}
                  min={0}
                  onChange={(value) => updateCommon('max_cache_mb', value)}
                />
                <ConfigNumberField
                  label="好友请求窗口（秒）"
                  value={settings.common.friend_request_window_sec}
                  min={0}
                  onChange={(value) => updateCommon('friend_request_window_sec', value)}
                />
                <div className="field-stack config-span-2">
                  <span className="field-label">NapCat 反向 WS 地址</span>
                  <Input
                    placeholder="0.0.0.0:3001/oqqwall/ws"
                    value={settings.common.napcat_base_url}
                    onChange={(event) => updateCommon('napcat_base_url', event.target.value)}
                  />
                </div>
                <div className="field-stack config-span-2">
                  <span className="field-label">NapCat Access Token</span>
                  <Input
                    placeholder="留空表示不在 config.json 中写入"
                    value={settings.common.napcat_access_token}
                    onChange={(event) => updateCommon('napcat_access_token', event.target.value)}
                  />
                </div>
                <div className="field-stack config-span-2">
                  <span className="field-label">好友通过自动私信</span>
                  <TextArea
                    value={settings.common.friend_add_message}
                    onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                      updateCommon('friend_add_message', event.target.value)
                    }
                  />
                </div>
                <div className="field-stack config-switch-row">
                  <span className="field-label">私密空间时 @ 投稿人</span>
                  <Switch
                    isSelected={settings.common.at_unprived_sender}
                    size="sm"
                    onChange={(value) => updateCommon('at_unprived_sender', value)}
                  >
                    {settings.common.at_unprived_sender ? '已启用' : '已关闭'}
                  </Switch>
                </div>
              </div>

              <div className="config-card-grid">
                <div className="config-subcard">
                  <div className="settings-section-head">
                    <span className="field-label">Web API</span>
                  </div>
                  <div className="settings-panel">
                    <div className="field-stack config-switch-row">
                      <span className="field-label">启用 Web API</span>
                      <Switch
                        isSelected={settings.common.web_api_enabled}
                        size="sm"
                        onChange={(value) => updateCommon('web_api_enabled', value)}
                      >
                        {settings.common.web_api_enabled ? '已启用' : '已关闭'}
                      </Switch>
                    </div>
                    <ConfigNumberField
                      label="Web API 端口"
                      value={settings.common.web_api_port}
                      min={1}
                      onChange={(value) => updateCommon('web_api_port', value)}
                    />
                    <div className="field-stack">
                      <span className="field-label">根令牌</span>
                      <Input
                        placeholder="建议使用 32 位以上随机串"
                        value={settings.common.web_api_root_token}
                        onChange={(event) => updateCommon('web_api_root_token', event.target.value)}
                      />
                    </div>
                  </div>
                </div>

                <div className="config-subcard">
                  <div className="settings-section-head">
                    <span className="field-label">Web 审核面板</span>
                  </div>
                  <div className="settings-panel">
                    <div className="field-stack config-switch-row">
                      <span className="field-label">启用 WebUI</span>
                      <Switch
                        isSelected={settings.common.webview_enabled}
                        size="sm"
                        onChange={(value) => updateCommon('webview_enabled', value)}
                      >
                        {settings.common.webview_enabled ? '已启用' : '已关闭'}
                      </Switch>
                    </div>
                    <div className="field-stack">
                      <span className="field-label">监听地址</span>
                      <Input
                        placeholder="0.0.0.0"
                        value={settings.common.webview_host}
                        onChange={(event) => updateCommon('webview_host', event.target.value)}
                      />
                    </div>
                    <ConfigNumberField
                      label="面板端口"
                      value={settings.common.webview_port}
                      min={1}
                      onChange={(value) => updateCommon('webview_port', value)}
                    />
                    <ConfigNumberField
                      label="会话有效期（秒）"
                      value={settings.common.webview_session_ttl_sec}
                      min={300}
                      onChange={(value) => updateCommon('webview_session_ttl_sec', value)}
                    />
                  </div>
                </div>

                <div className="config-subcard">
                  <div className="settings-section-head">
                    <span className="field-label">遥测</span>
                  </div>
                  <div className="settings-panel">
                    <div className="field-stack config-switch-row">
                      <span className="field-label">启用遥测</span>
                      <Switch
                        isSelected={settings.common.telemetry_enabled}
                        size="sm"
                        onChange={(value) => updateCommon('telemetry_enabled', value)}
                      >
                        {settings.common.telemetry_enabled ? '已启用' : '已关闭'}
                      </Switch>
                    </div>
                    <div className="field-stack">
                      <span className="field-label">本地目录</span>
                      <Input
                        placeholder="telemetry"
                        value={settings.common.telemetry_local_dir}
                        onChange={(event) => updateCommon('telemetry_local_dir', event.target.value)}
                      />
                    </div>
                    <div className="field-stack config-switch-row">
                      <span className="field-label">启用上传</span>
                      <Switch
                        isSelected={settings.common.telemetry_upload_enabled}
                        size="sm"
                        onChange={(value) => updateCommon('telemetry_upload_enabled', value)}
                      >
                        {settings.common.telemetry_upload_enabled ? '已启用' : '已关闭'}
                      </Switch>
                    </div>
                    <ConfigNumberField
                      label="上传间隔（秒）"
                      value={settings.common.telemetry_upload_interval_sec}
                      min={1}
                      onChange={(value) => updateCommon('telemetry_upload_interval_sec', value)}
                    />
                    <ConfigNumberField
                      label="批量大小"
                      value={settings.common.telemetry_upload_batch_size}
                      min={1}
                      onChange={(value) => updateCommon('telemetry_upload_batch_size', value)}
                    />
                    <ConfigNumberField
                      label="追加消息上限"
                      value={settings.common.telemetry_max_append_messages}
                      min={1}
                      onChange={(value) => updateCommon('telemetry_max_append_messages', value)}
                    />
                  </div>
                </div>
              </div>
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card">
          <Card.Header>
            <Card.Title>全局 WebUI 管理员</Card.Title>
          </Card.Header>
          <Card.Content>
            <ConfigAdminListEditor
              entries={settings.global_admins}
              emptyText="当前没有全局管理员。启用 Web 审核面板前，请至少保留一个管理员。"
              onAdd={addGlobalAdmin}
              onRemove={removeGlobalAdmin}
              onChange={updateGlobalAdmin}
            />
          </Card.Content>
        </Card>

        <Card className="panel-card">
          <Card.Header>
            <Card.Title>分组配置</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-section-head">
                <span className="field-hint">每个分组都会写回到 `config.json.groups`。</span>
                <div className="head-actions">
                  <Button size="sm" variant="secondary" onClick={addGroup}>
                    新增分组
                  </Button>
                  <Button size="sm" variant="secondary" onClick={removeCurrentGroup}>
                    删除当前分组
                  </Button>
                </div>
              </div>

              <div className="settings-stage-grid">
                {settings.groups.map((group) => (
                  <button
                    key={group.group_id}
                    type="button"
                    className={`settings-stage-button ${
                      activeGroup?.group_id === group.group_id ? 'is-active' : ''
                    }`}
                    onClick={() => setSelectedGroup(group.group_id)}
                  >
                    <span className="settings-stage-icon">
                      <LayoutGrid size={16} />
                    </span>
                    <strong>{group.group_id || '未命名分组'}</strong>
                    <span>审核群：{group.audit_group_id || '未填写'}</span>
                  </button>
                ))}
              </div>

              {activeGroup ? (
                <div className="settings-panel">
                  <div className="config-form-grid">
                    <div className="field-stack">
                      <span className="field-label">分组标识</span>
                      <Input
                        value={activeGroup.group_id}
                        onChange={(event) => updateGroupField('group_id', event.target.value)}
                      />
                    </div>
                    <div className="field-stack">
                      <span className="field-label">审核群号</span>
                      <Input
                        value={activeGroup.audit_group_id}
                        onChange={(event) => updateGroupField('audit_group_id', event.target.value)}
                      />
                    </div>
                    <ConfigNumberField
                      label="处理等待（秒）"
                      value={activeGroup.process_waittime_sec}
                      min={0}
                      onChange={(value) => updateGroupField('process_waittime_sec', value)}
                    />
                    <ConfigNumberField
                      label="最小发送间隔（毫秒）"
                      value={activeGroup.min_interval_ms}
                      min={0}
                      onChange={(value) => updateGroupField('min_interval_ms', value)}
                    />
                    <ConfigNumberField
                      label="队列编组上限"
                      value={activeGroup.max_post_stack}
                      min={1}
                      onChange={(value) => updateGroupField('max_post_stack', value)}
                    />
                    <ConfigNumberField
                      label="单条最大图片数"
                      value={activeGroup.max_image_number_one_post}
                      min={1}
                      onChange={(value) => updateGroupField('max_image_number_one_post', value)}
                    />
                    <ConfigNumberField
                      label="发送超时（毫秒）"
                      value={activeGroup.send_timeout_ms}
                      min={0}
                      onChange={(value) => updateGroupField('send_timeout_ms', value)}
                    />
                    <ConfigNumberField
                      label="发送重试次数"
                      value={activeGroup.send_max_attempts}
                      min={0}
                      onChange={(value) => updateGroupField('send_max_attempts', value)}
                    />
                    <ConfigNumberField
                      label="好友请求窗口（秒）"
                      value={activeGroup.friend_request_window_sec}
                      min={0}
                      onChange={(value) => updateGroupField('friend_request_window_sec', value)}
                    />
                    <div className="field-stack config-switch-row">
                      <span className="field-label">发稿时附带原图</span>
                      <Switch
                        isSelected={activeGroup.individual_image_in_posts}
                        size="sm"
                        onChange={(value) => updateGroupField('individual_image_in_posts', value)}
                      >
                        {activeGroup.individual_image_in_posts ? '已启用' : '已关闭'}
                      </Switch>
                    </div>
                    <div className="field-stack config-span-2">
                      <span className="field-label">NapCat 反向 WS 地址</span>
                      <Input
                        value={activeGroup.napcat_base_url}
                        onChange={(event) => updateGroupField('napcat_base_url', event.target.value)}
                      />
                    </div>
                    <div className="field-stack config-span-2">
                      <span className="field-label">NapCat Access Token</span>
                      <Input
                        value={activeGroup.napcat_access_token}
                        onChange={(event) =>
                          updateGroupField('napcat_access_token', event.target.value)
                        }
                      />
                    </div>
                    <div className="field-stack config-span-2">
                      <span className="field-label">渲染水印文本</span>
                      <Input
                        value={activeGroup.watermark_text}
                        onChange={(event) => updateGroupField('watermark_text', event.target.value)}
                      />
                    </div>
                    <div className="field-stack config-span-2">
                      <span className="field-label">好友通过自动私信</span>
                      <TextArea
                        value={activeGroup.friend_add_message}
                        onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                          updateGroupField('friend_add_message', event.target.value)
                        }
                      />
                    </div>
                  </div>

                  <div className="config-card-grid">
                    <div className="config-subcard">
                      <StringListEditor
                        label="账号列表"
                        hint="首项为主账号，系统会按顺序选可用账号发送。"
                        addLabel="新增账号"
                        placeholder="请输入 QQ 号"
                        values={activeGroup.accounts}
                        onAdd={() => addGroupStringListItem('accounts')}
                        onChange={(index, value) => updateGroupStringList('accounts', index, value)}
                        onRemove={(index) => removeGroupStringListItem('accounts', index)}
                      />
                    </div>
                    <div className="config-subcard">
                      <StringListEditor
                        label="定时发送时间"
                        hint="格式为 HH:MM，例如 08:30。留空表示不启用定时发送。"
                        addLabel="新增时间"
                        placeholder="08:30"
                        values={activeGroup.send_schedule}
                        onAdd={() => addGroupStringListItem('send_schedule')}
                        onChange={(index, value) =>
                          updateGroupStringList('send_schedule', index, value)
                        }
                        onRemove={(index) => removeGroupStringListItem('send_schedule', index)}
                      />
                    </div>
                  </div>

                  <div className="config-card-grid">
                    <div className="config-subcard">
                      <MappingListEditor
                        label="快捷回复映射"
                        hint="格式：指令 -> 回复文本。"
                        leftPlaceholder="指令"
                        rightPlaceholder="回复文本"
                        entries={activeGroup.quick_replies}
                        onAdd={() => addGroupMapping('quick_replies')}
                        onChange={(index, key, value) =>
                          updateGroupMapping('quick_replies', index, key, value)
                        }
                        onRemove={(index) => removeGroupMapping('quick_replies', index)}
                      />
                    </div>
                    <div className="config-subcard">
                      <MappingListEditor
                        label="审核快捷指令"
                        hint="格式：指令 -> 步骤 DSL，例如 匿 | 是。"
                        leftPlaceholder="指令"
                        rightPlaceholder="步骤 DSL"
                        entries={activeGroup.review_shortcuts}
                        onAdd={() => addGroupMapping('review_shortcuts')}
                        onChange={(index, key, value) =>
                          updateGroupMapping('review_shortcuts', index, key, value)
                        }
                        onRemove={(index) => removeGroupMapping('review_shortcuts', index)}
                      />
                    </div>
                    <div className="config-subcard">
                      <MappingListEditor
                        label="全局快捷指令"
                        hint="格式：指令 -> 全局动作 DSL。"
                        leftPlaceholder="指令"
                        rightPlaceholder="步骤 DSL"
                        entries={activeGroup.global_shortcuts}
                        onAdd={() => addGroupMapping('global_shortcuts')}
                        onChange={(index, key, value) =>
                          updateGroupMapping('global_shortcuts', index, key, value)
                        }
                        onRemove={(index) => removeGroupMapping('global_shortcuts', index)}
                      />
                    </div>
                  </div>

                  <div className="config-subcard">
                    <AgentCommandWorkbench
                      commands={activeGroup.agent_commands}
                      variables={settings.agent_command_variables}
                      onChange={(commands) => updateGroupField('agent_commands', commands)}
                    />
                  </div>

                  <div className="config-subcard">
                    <ConfigAdminListEditor
                      entries={activeGroup.webview_admins}
                      emptyText="当前分组还没有专属 WebUI 管理员。"
                      onAdd={addGroupAdmin}
                      onRemove={removeGroupAdmin}
                      onChange={updateGroupAdmin}
                    />
                  </div>
                </div>
              ) : (
                <div className="settings-empty">
                  <LayoutGrid size={20} />
                  <span>当前没有分组配置。</span>
                </div>
              )}
            </div>
          </Card.Content>
        </Card>
      </section>
    </div>
  )
  */
  return <RuntimeConfigWorkbench notify={notify} />
}

function RuntimeConfigWorkbench({
  notify,
  mode = 'full',
}: {
  notify: (kind: ToastKind, text: string) => void
  mode?: RuntimeConfigWorkbenchMode
}) {
  const [settings, setSettings] = useState<AppConfigSettingsResponse | null>(null)
  const [selectedGroup, setSelectedGroup] = useState('')
  const [selectedMenu, setSelectedMenu] = useState<ConfigMenuKey>('common_runtime')
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    void loadSettings()
  }, [])

  function applySettings(result: AppConfigSettingsResponse) {
    setSettings(result)
    setSelectedGroup((prev) =>
      result.groups.some((group) => group.group_id === prev)
        ? prev
        : (result.groups[0]?.group_id ?? '')
    )
  }

  async function loadSettings() {
    setLoading(true)
    try {
      applySettings(await api<AppConfigSettingsResponse>('/api/settings/config'))
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  async function saveSettings() {
    if (!settings) return
    setSaving(true)
    try {
      const result = await api<AppConfigSettingsResponse>('/api/settings/config', {
        method: 'POST',
        body: JSON.stringify({
          common: settings.common,
          global_admins: settings.global_admins,
          groups: settings.groups,
        }),
      })
      applySettings(result)
      notify('success', '运行配置已保存到 config.json')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setSaving(false)
    }
  }

  function updateCommon<K extends keyof AppConfigCommonSettings>(
    key: K,
    value: AppConfigCommonSettings[K]
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        common: {
          ...prev.common,
          [key]: value,
        },
      }
    })
  }

  function updateSelectedGroup(
    updater: (group: AppConfigGroupSettings) => AppConfigGroupSettings
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        groups: prev.groups.map((group) =>
          group.group_id === selectedGroup ? updater(group) : group
        ),
      }
    })
  }

  function updateGroupField<K extends keyof AppConfigGroupSettings>(
    key: K,
    value: AppConfigGroupSettings[K]
  ) {
    if (key === 'group_id') {
      setSelectedGroup(String(value))
    }
    updateSelectedGroup((group) => ({
      ...group,
      [key]: value,
    }))
  }

  function updateGroupStringList(field: 'accounts' | 'send_schedule', index: number, value: string) {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: group[field].map((item, itemIndex) => (itemIndex === index ? value : item)),
    }))
  }

  function addGroupStringListItem(field: 'accounts' | 'send_schedule') {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: [...group[field], ''],
    }))
  }

  function removeGroupStringListItem(field: 'accounts' | 'send_schedule', index: number) {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: group[field].filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function updateGroupMapping(
    field: 'quick_replies' | 'review_shortcuts' | 'global_shortcuts',
    index: number,
    key: keyof MappingEntry,
    value: string
  ) {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: group[field].map((item, itemIndex) =>
        itemIndex === index ? { ...item, [key]: value } : item
      ),
    }))
  }

  function addGroupMapping(field: 'quick_replies' | 'review_shortcuts' | 'global_shortcuts') {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: [...group[field], { key: '', value: '' }],
    }))
  }

  function removeGroupMapping(
    field: 'quick_replies' | 'review_shortcuts' | 'global_shortcuts',
    index: number
  ) {
    updateSelectedGroup((group) => ({
      ...group,
      [field]: group[field].filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function updateGlobalAdmin(index: number, key: keyof ConfigAdminEntry, value: string | boolean) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        global_admins: prev.global_admins.map((item, itemIndex) =>
          itemIndex === index ? { ...item, [key]: value } : item
        ),
      }
    })
  }

  function addGlobalAdmin() {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        global_admins: [
          ...prev.global_admins,
          { id: '', username: '', password: '', password_set: false },
        ],
      }
    })
  }

  function removeGlobalAdmin(index: number) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        global_admins: prev.global_admins.filter((_, itemIndex) => itemIndex !== index),
      }
    })
  }

  function updateGroupAdmin(index: number, key: keyof ConfigAdminEntry, value: string | boolean) {
    updateSelectedGroup((group) => ({
      ...group,
      webview_admins: group.webview_admins.map((item, itemIndex) =>
        itemIndex === index ? { ...item, [key]: value } : item
      ),
    }))
  }

  function addGroupAdmin() {
    updateSelectedGroup((group) => ({
      ...group,
      webview_admins: [
        ...group.webview_admins,
        { id: '', username: '', password: '', password_set: false },
      ],
    }))
  }

  function removeGroupAdmin(index: number) {
    updateSelectedGroup((group) => ({
      ...group,
      webview_admins: group.webview_admins.filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function addGroup() {
    let nextGroupId = ''
    setSettings((prev) => {
      if (!prev) return prev
      nextGroupId = buildNextGroupId(prev.groups)
      return {
        ...prev,
        groups: [...prev.groups, buildDefaultConfigGroup(nextGroupId, prev.common)],
      }
    })
    if (nextGroupId) setSelectedGroup(nextGroupId)
  }

  function removeCurrentGroup() {
    if (!settings) return
    if (settings.groups.length <= 1) {
      notify('error', '至少保留一个分组配置')
      return
    }
    const activeGroup = settings.groups.find((group) => group.group_id === selectedGroup)
    if (!activeGroup) return
    if (!window.confirm(`确定删除分组 ${activeGroup.group_id} 吗？`)) {
      return
    }
    const remainingGroups = settings.groups.filter((group) => group.group_id !== activeGroup.group_id)
    setSettings((prev) => (prev ? { ...prev, groups: remainingGroups } : prev))
    setSelectedGroup(remainingGroups[0]?.group_id ?? '')
  }

  if (loading && !settings) {
    return <EmptyPanel icon={<Spinner />} text="正在加载运行配置" />
  }

  if (!settings) {
    return (
      <div className="workspace">
        <header className="page-head">
          <div>
            <h1>运行配置</h1>
            <p>在 WebUI 中用 GUI 方式维护 config.json。</p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            重试
          </Button>
        </header>
        <Card className="panel-card">
          <Card.Content>
            <EmptyPanel icon={<LayoutGrid size={28} />} text="当前没有可用的运行配置数据" />
          </Card.Content>
        </Card>
      </div>
    )
  }

  const groups = settings.groups
  const activeGroup = groups.find((group) => group.group_id === selectedGroup) ?? groups[0] ?? null
  if (mode === 'agent') {
    return (
      <div className="workspace settings-workspace agent-workspace">
        <header className="page-head">
          <div>
            <h1>积木 Agent</h1>
            <p>把左侧积木拖到右侧工作区，在网页上直接拼接成用户指令流。</p>
          </div>
          <div className="head-actions">
            <Button
              size="sm"
              variant="secondary"
              isDisabled={loading || saving}
              onClick={() => void loadSettings()}
            >
              <RefreshCcw size={16} />
              刷新
            </Button>
            <Button size="sm" isDisabled={loading || saving} onClick={() => void saveSettings()}>
              {saving ? <Spinner size="sm" /> : <Check size={16} />}
              保存
            </Button>
          </div>
        </header>

        <section className="settings-layout agent-layout">
          {renderGroupSelectionCard()}
          <div className="settings-content-stack">
            <Card className="panel-card agent-workbench-card">
              <Card.Header>
                <Card.Title>拖拽拼接工作台</Card.Title>
              </Card.Header>
              <Card.Content>
                {activeGroup ? (
                  <AgentCommandWorkbench
                    commands={activeGroup.agent_commands}
                    variables={settings.agent_command_variables}
                    onChange={(commands) => updateGroupField('agent_commands', commands)}
                  />
                ) : (
                  <EmptyPanel icon={<LayoutGrid size={28} />} text="当前没有可用的分组配置" />
                )}
              </Card.Content>
            </Card>
          </div>
        </section>
      </div>
    )
  }

  function renderGroupSelectionCard(extra?: ReactNode) {
    return (
      <Card className="panel-card">
        <Card.Content>
          <div className="agent-group-selector">
            <div className="field-stack">
              <span className="field-label">当前分组</span>
              <HeroSelect
                className="control settings-group-select"
                ariaLabel="选择配置分组"
                selectedKey={activeGroup?.group_id ?? ''}
                options={groups.map((group) => ({
                  value: group.group_id,
                  label: group.group_id || '未命名分组',
                }))}
                onSelect={(value) => setSelectedGroup(value)}
              />
            </div>
            <div className="settings-toolbar">
              <div className="head-actions">
                <Button size="sm" variant="secondary" onClick={addGroup}>
                  新增分组
                </Button>
                <Button size="sm" variant="secondary" onClick={removeCurrentGroup}>
                  删除当前分组
                </Button>
              </div>
              {extra}
            </div>
          </div>
        </Card.Content>
      </Card>
    )
  }

  return (
    <div className="workspace settings-workspace">
      <header className="page-head">
        <div>
          <h1>运行配置</h1>
          <p>把不同运行板块拆到独立菜单里，便于单独配置。</p>
        </div>
        <div className="head-actions">
          <Button size="sm" variant="secondary" isDisabled={loading || saving} onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            刷新
          </Button>
          <Button size="sm" isDisabled={loading || saving} onClick={() => void saveSettings()}>
            {saving ? <Spinner size="sm" /> : <Check size={16} />}
            保存
          </Button>
        </div>
      </header>

      <section className="settings-layout">
        <SettingsMenuCard
          title="运行配置页面"
          options={buildRuntimeConfigPages()}
          activeKey={selectedMenu}
          onSelect={setSelectedMenu}
        />

        <div className="settings-content-stack">
          {selectedMenu === 'common_runtime' ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>基础运行</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="settings-panel">
                  <div className="config-form-grid">
                    <ConfigNumberField
                      label="处理等待（秒）"
                      value={settings.common.process_waittime_sec}
                      min={0}
                      onChange={(value) => updateCommon('process_waittime_sec', value)}
                    />
                    <ConfigNumberField
                      label="最小发送间隔（毫秒）"
                      value={settings.common.min_interval_ms}
                      min={0}
                      onChange={(value) => updateCommon('min_interval_ms', value)}
                    />
                    <ConfigNumberField
                      label="公共最大图片数"
                      value={settings.common.max_image_number_one_post}
                      min={1}
                      onChange={(value) => updateCommon('max_image_number_one_post', value)}
                    />
                    <ConfigNumberField
                      label="发送超时（毫秒）"
                      value={settings.common.send_timeout_ms}
                      min={0}
                      onChange={(value) => updateCommon('send_timeout_ms', value)}
                    />
                    <ConfigNumberField
                      label="发送重试次数"
                      value={settings.common.send_max_attempts}
                      min={0}
                      onChange={(value) => updateCommon('send_max_attempts', value)}
                    />
                    <ConfigNumberField
                      label="时区偏移（分钟）"
                      value={settings.common.tz_offset_minutes}
                      allowNegative
                      onChange={(value) => updateCommon('tz_offset_minutes', value)}
                    />
                    <ConfigNumberField
                      label="缓存上限（MB）"
                      value={settings.common.max_cache_mb}
                      min={0}
                      onChange={(value) => updateCommon('max_cache_mb', value)}
                    />
                    <ConfigNumberField
                      label="好友请求窗口（秒）"
                      value={settings.common.friend_request_window_sec}
                      min={0}
                      onChange={(value) => updateCommon('friend_request_window_sec', value)}
                    />
                    <div className="field-stack config-span-2">
                      <span className="field-label">NapCat 反向 WS 地址</span>
                      <Input
                        value={settings.common.napcat_base_url}
                        onChange={(event) => updateCommon('napcat_base_url', event.target.value)}
                      />
                    </div>
                    <div className="field-stack config-span-2">
                      <span className="field-label">NapCat Access Token</span>
                      <Input
                        value={settings.common.napcat_access_token}
                        onChange={(event) =>
                          updateCommon('napcat_access_token', event.target.value)
                        }
                      />
                    </div>
                    <div className="field-stack config-span-2">
                      <span className="field-label">好友通过自动私信</span>
                      <TextArea
                        value={settings.common.friend_add_message}
                        onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                          updateCommon('friend_add_message', event.target.value)
                        }
                      />
                    </div>
                    <div className="field-stack config-switch-row">
                      <span className="field-label">私密空间时 @ 投稿人</span>
                      <Switch
                        isSelected={settings.common.at_unprived_sender}
                        size="sm"
                        onChange={(value) => updateCommon('at_unprived_sender', value)}
                      >
                        {settings.common.at_unprived_sender ? '已启用' : '已关闭'}
                      </Switch>
                    </div>
                  </div>
                </div>
              </Card.Content>
            </Card>
          ) : null}

          {selectedMenu === 'common_services' ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>服务端口</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="config-card-grid">
                  <div className="config-subcard">
                    <div className="settings-section-head">
                      <span className="field-label">Web API</span>
                    </div>
                    <div className="settings-panel">
                      <div className="field-stack config-switch-row">
                        <span className="field-label">启用 Web API</span>
                        <Switch
                          isSelected={settings.common.web_api_enabled}
                          size="sm"
                          onChange={(value) => updateCommon('web_api_enabled', value)}
                        >
                          {settings.common.web_api_enabled ? '已启用' : '已关闭'}
                        </Switch>
                      </div>
                      <ConfigNumberField
                        label="Web API 端口"
                        value={settings.common.web_api_port}
                        min={1}
                        onChange={(value) => updateCommon('web_api_port', value)}
                      />
                      <div className="field-stack">
                        <span className="field-label">根令牌</span>
                        <Input
                          value={settings.common.web_api_root_token}
                          onChange={(event) =>
                            updateCommon('web_api_root_token', event.target.value)
                          }
                        />
                      </div>
                    </div>
                  </div>

                  <div className="config-subcard">
                    <div className="settings-section-head">
                      <span className="field-label">Web 审核面板</span>
                    </div>
                    <div className="settings-panel">
                      <div className="field-stack config-switch-row">
                        <span className="field-label">启用 WebUI</span>
                        <Switch
                          isSelected={settings.common.webview_enabled}
                          size="sm"
                          onChange={(value) => updateCommon('webview_enabled', value)}
                        >
                          {settings.common.webview_enabled ? '已启用' : '已关闭'}
                        </Switch>
                      </div>
                      <div className="field-stack">
                        <span className="field-label">监听地址</span>
                        <Input
                          value={settings.common.webview_host}
                          onChange={(event) => updateCommon('webview_host', event.target.value)}
                        />
                      </div>
                      <ConfigNumberField
                        label="面板端口"
                        value={settings.common.webview_port}
                        min={1}
                        onChange={(value) => updateCommon('webview_port', value)}
                      />
                      <ConfigNumberField
                        label="会话有效期（秒）"
                        value={settings.common.webview_session_ttl_sec}
                        min={300}
                        onChange={(value) => updateCommon('webview_session_ttl_sec', value)}
                      />
                    </div>
                  </div>
                </div>
              </Card.Content>
            </Card>
          ) : null}

          {selectedMenu === 'common_telemetry' ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>遥测</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="config-card-grid">
                  <div className="config-subcard">
                    <div className="settings-panel">
                      <div className="field-stack config-switch-row">
                        <span className="field-label">启用遥测</span>
                        <Switch
                          isSelected={settings.common.telemetry_enabled}
                          size="sm"
                          onChange={(value) => updateCommon('telemetry_enabled', value)}
                        >
                          {settings.common.telemetry_enabled ? '已启用' : '已关闭'}
                        </Switch>
                      </div>
                      <div className="field-stack">
                        <span className="field-label">本地目录</span>
                        <Input
                          value={settings.common.telemetry_local_dir}
                          onChange={(event) =>
                            updateCommon('telemetry_local_dir', event.target.value)
                          }
                        />
                      </div>
                      <div className="field-stack config-switch-row">
                        <span className="field-label">启用上传</span>
                        <Switch
                          isSelected={settings.common.telemetry_upload_enabled}
                          size="sm"
                          onChange={(value) => updateCommon('telemetry_upload_enabled', value)}
                        >
                          {settings.common.telemetry_upload_enabled ? '已启用' : '已关闭'}
                        </Switch>
                      </div>
                      <ConfigNumberField
                        label="上传间隔（秒）"
                        value={settings.common.telemetry_upload_interval_sec}
                        min={1}
                        onChange={(value) =>
                          updateCommon('telemetry_upload_interval_sec', value)
                        }
                      />
                      <ConfigNumberField
                        label="上传批量大小"
                        value={settings.common.telemetry_upload_batch_size}
                        min={1}
                        onChange={(value) =>
                          updateCommon('telemetry_upload_batch_size', value)
                        }
                      />
                      <ConfigNumberField
                        label="追加消息上限"
                        value={settings.common.telemetry_max_append_messages}
                        min={1}
                        onChange={(value) =>
                          updateCommon('telemetry_max_append_messages', value)
                        }
                      />
                    </div>
                  </div>
                </div>
              </Card.Content>
            </Card>
          ) : null}

          {selectedMenu === 'global_admins' ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>全局 WebUI 管理员</Card.Title>
              </Card.Header>
              <Card.Content>
                <ConfigAdminListEditor
                  entries={settings.global_admins}
                  emptyText="当前还没有全局管理员。"
                  onAdd={addGlobalAdmin}
                  onRemove={removeGlobalAdmin}
                  onChange={updateGlobalAdmin}
                />
              </Card.Content>
            </Card>
          ) : null}

          {selectedMenu.startsWith('group_') && activeGroup ? renderGroupSelectionCard() : null}

          {selectedMenu === 'group_basic' && activeGroup ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>分组基础</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="config-form-grid">
                  <div className="field-stack">
                    <span className="field-label">分组标识</span>
                    <Input
                      value={activeGroup.group_id}
                      onChange={(event) => updateGroupField('group_id', event.target.value)}
                    />
                  </div>
                  <div className="field-stack">
                    <span className="field-label">审核群号</span>
                    <Input
                      value={activeGroup.audit_group_id}
                      onChange={(event) => updateGroupField('audit_group_id', event.target.value)}
                    />
                  </div>
                  <ConfigNumberField
                    label="处理等待（秒）"
                    value={activeGroup.process_waittime_sec}
                    min={0}
                    onChange={(value) => updateGroupField('process_waittime_sec', value)}
                  />
                  <ConfigNumberField
                    label="最小发送间隔（毫秒）"
                    value={activeGroup.min_interval_ms}
                    min={0}
                    onChange={(value) => updateGroupField('min_interval_ms', value)}
                  />
                  <ConfigNumberField
                    label="发送超时（毫秒）"
                    value={activeGroup.send_timeout_ms}
                    min={0}
                    onChange={(value) => updateGroupField('send_timeout_ms', value)}
                  />
                  <ConfigNumberField
                    label="发送重试次数"
                    value={activeGroup.send_max_attempts}
                    min={0}
                    onChange={(value) => updateGroupField('send_max_attempts', value)}
                  />
                  <ConfigNumberField
                    label="好友请求窗口（秒）"
                    value={activeGroup.friend_request_window_sec}
                    min={0}
                    onChange={(value) => updateGroupField('friend_request_window_sec', value)}
                  />
                  <div className="field-stack config-span-2">
                    <span className="field-label">NapCat 反向 WS 地址</span>
                    <Input
                      value={activeGroup.napcat_base_url}
                      onChange={(event) => updateGroupField('napcat_base_url', event.target.value)}
                    />
                  </div>
                  <div className="field-stack config-span-2">
                    <span className="field-label">NapCat Access Token</span>
                    <Input
                      value={activeGroup.napcat_access_token}
                      onChange={(event) =>
                        updateGroupField('napcat_access_token', event.target.value)
                      }
                    />
                  </div>
                  <div className="field-stack config-span-2">
                    <span className="field-label">渲染水印文本</span>
                    <Input
                      value={activeGroup.watermark_text}
                      onChange={(event) => updateGroupField('watermark_text', event.target.value)}
                    />
                  </div>
                  <div className="field-stack config-span-2">
                    <span className="field-label">好友通过自动私信</span>
                    <TextArea
                      value={activeGroup.friend_add_message}
                      onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                        updateGroupField('friend_add_message', event.target.value)
                      }
                    />
                  </div>
                </div>
              </Card.Content>
            </Card>
          ) : null}

          {selectedMenu === 'group_delivery' && activeGroup ? (
            <div className="config-card-grid">
              <div className="config-subcard">
                <div className="settings-panel">
                  <ConfigNumberField
                    label="队列编组上限"
                    value={activeGroup.max_post_stack}
                    min={1}
                    onChange={(value) => updateGroupField('max_post_stack', value)}
                  />
                  <ConfigNumberField
                    label="单条最大图片数"
                    value={activeGroup.max_image_number_one_post}
                    min={1}
                    onChange={(value) =>
                      updateGroupField('max_image_number_one_post', value)
                    }
                  />
                  <div className="field-stack config-switch-row">
                    <span className="field-label">发稿时附带原图</span>
                    <Switch
                      isSelected={activeGroup.individual_image_in_posts}
                      size="sm"
                      onChange={(value) =>
                        updateGroupField('individual_image_in_posts', value)
                      }
                    >
                      {activeGroup.individual_image_in_posts ? '已启用' : '已关闭'}
                    </Switch>
                  </div>
                </div>
              </div>

              <div className="config-subcard">
                <StringListEditor
                  label="账号列表"
                  hint="首项为主账号，系统会按顺序选择可用账号发稿。"
                  addLabel="新增账号"
                  placeholder="请输入 QQ 号"
                  values={activeGroup.accounts}
                  onAdd={() => addGroupStringListItem('accounts')}
                  onChange={(index, value) => updateGroupStringList('accounts', index, value)}
                  onRemove={(index) => removeGroupStringListItem('accounts', index)}
                />
              </div>

              <div className="config-subcard">
                <StringListEditor
                  label="定时发稿时间"
                  hint="格式 HH:MM，例如 08:30。"
                  addLabel="新增时间"
                  placeholder="08:30"
                  values={activeGroup.send_schedule}
                  onAdd={() => addGroupStringListItem('send_schedule')}
                  onChange={(index, value) => updateGroupStringList('send_schedule', index, value)}
                  onRemove={(index) => removeGroupStringListItem('send_schedule', index)}
                />
              </div>
            </div>
          ) : null}

          {selectedMenu === 'group_quick_replies' && activeGroup ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>快捷回复</Card.Title>
              </Card.Header>
              <Card.Content>
                <MappingListEditor
                  label="快捷回复映射"
                  hint="格式：指令 -> 回复文本。"
                  leftPlaceholder="指令"
                  rightPlaceholder="回复文本"
                  entries={activeGroup.quick_replies}
                  onAdd={() => addGroupMapping('quick_replies')}
                  onChange={(index, key, value) =>
                    updateGroupMapping('quick_replies', index, key, value)
                  }
                  onRemove={(index) => removeGroupMapping('quick_replies', index)}
                />
              </Card.Content>
            </Card>
          ) : null}

          {selectedMenu === 'group_shortcuts' && activeGroup ? (
            <div className="config-card-grid">
              <div className="config-subcard">
                <MappingListEditor
                  label="审核快捷指令"
                  hint="格式：指令 -> 步骤 DSL。"
                  leftPlaceholder="指令"
                  rightPlaceholder="步骤 DSL"
                  entries={activeGroup.review_shortcuts}
                  onAdd={() => addGroupMapping('review_shortcuts')}
                  onChange={(index, key, value) =>
                    updateGroupMapping('review_shortcuts', index, key, value)
                  }
                  onRemove={(index) => removeGroupMapping('review_shortcuts', index)}
                />
              </div>
              <div className="config-subcard">
                <MappingListEditor
                  label="全局快捷指令"
                  hint="格式：指令 -> 全局动作 DSL。"
                  leftPlaceholder="指令"
                  rightPlaceholder="步骤 DSL"
                  entries={activeGroup.global_shortcuts}
                  onAdd={() => addGroupMapping('global_shortcuts')}
                  onChange={(index, key, value) =>
                    updateGroupMapping('global_shortcuts', index, key, value)
                  }
                  onRemove={(index) => removeGroupMapping('global_shortcuts', index)}
                />
              </div>
            </div>
          ) : null}
          {selectedMenu === 'group_agent_commands' && activeGroup ? (
            <Card className="panel-card agent-legacy-card">
              <Card.Header>
                <Card.Title>Agent 指令</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="settings-panel">
                  <div className="settings-empty">
                    <FileText size={20} />
                    <span>Agent 积木编辑器已移动到左侧导航的独立页面。</span>
                  </div>
                  <p className="field-hint">在独立页面中可以拖拽积木、拼接，并保存到当前分组。</p>
                </div>
              </Card.Content>
            </Card>
          ) : null}

          {selectedMenu === 'group_admins' && activeGroup ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>分组 WebUI 管理员</Card.Title>
              </Card.Header>
              <Card.Content>
                <ConfigAdminListEditor
                  entries={activeGroup.webview_admins}
                  emptyText="当前分组还没有专属 WebUI 管理员。"
                  onAdd={addGroupAdmin}
                  onRemove={removeGroupAdmin}
                  onChange={updateGroupAdmin}
                />
              </Card.Content>
            </Card>
          ) : null}
        </div>
      </section>
    </div>
  )
}

function ConfigNumberField({
  label,
  value,
  onChange,
  min,
  allowNegative = false,
}: {
  label: string
  value: number
  onChange: (value: number) => void
  min?: number
  allowNegative?: boolean
}) {
  return (
    <div className="field-stack">
      <span className="field-label">{label}</span>
      <Input
        type="number"
        min={min}
        value={String(value)}
        onChange={(event) =>
          onChange(parseIntegerInput(event.target.value, { allowNegative, fallback: value }))
        }
      />
    </div>
  )
}

function StringListEditor({
  label,
  hint,
  addLabel,
  placeholder,
  values,
  onAdd,
  onChange,
  onRemove,
}: {
  label: string
  hint?: string
  addLabel: string
  placeholder: string
  values: string[]
  onAdd: () => void
  onChange: (index: number, value: string) => void
  onRemove: (index: number) => void
}) {
  return (
    <div className="settings-panel">
      <div className="settings-section-head">
        <span className="field-label">{label}</span>
        <Button size="sm" variant="secondary" onClick={onAdd}>
          {addLabel}
        </Button>
      </div>
      {hint ? <span className="field-hint">{hint}</span> : null}
      {values.length ? (
        <div className="settings-image-list">
          {values.map((item, index) => (
            <div key={`${label}-${index}`} className="settings-image-item">
              <div className="settings-image-row">
                <Input
                  className="settings-image-input"
                  placeholder={placeholder}
                  value={item}
                  onChange={(event) => onChange(index, event.target.value)}
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

function MappingListEditor({
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

function ConfigAdminListEditor({
  entries,
  emptyText,
  onAdd,
  onRemove,
  onChange,
}: {
  entries: ConfigAdminEntry[]
  emptyText: string
  onAdd: () => void
  onRemove: (index: number) => void
  onChange: (index: number, key: keyof ConfigAdminEntry, value: string | boolean) => void
}) {
  return (
    <div className="settings-panel">
      <div className="settings-section-head">
        <span className="field-hint">
          密码列支持直接填写明文；如果这项管理员已经有密码，留空表示不修改原密码。
        </span>
        <Button size="sm" variant="secondary" onClick={onAdd}>
          新增管理员
        </Button>
      </div>
      {entries.length ? (
        <div className="settings-variable-list">
          {entries.map((entry, index) => (
            <div key={entry.id || `admin-${index}`} className="settings-variable-card">
              <div className="config-admin-grid">
                <Input
                  placeholder="用户名"
                  value={entry.username}
                  onChange={(event) => onChange(index, 'username', event.target.value)}
                />
                <Input
                  placeholder={entry.password_set ? '留空表示保持原密码' : '请输入密码'}
                  type="password"
                  value={entry.password}
                  onChange={(event) => onChange(index, 'password', event.target.value)}
                />
                <Button
                  size="sm"
                  variant="tertiary"
                  isIconOnly
                  aria-label={`删除管理员 ${index + 1}`}
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
          <ShieldCheck size={20} />
          <span>{emptyText}</span>
        </div>
      )}
    </div>
  )
}

function AgentCommandWorkbench({
  commands,
  variables,
  onChange,
}: {
  commands: AppConfigAgentCommand[]
  variables: AppConfigSettingsResponse['agent_command_variables']
  onChange: (commands: AppConfigAgentCommand[]) => void
}) {
  const fieldRefs = useRef<Record<string, AgentCommandFieldNode | null>>({})
  const [activeFieldKey, setActiveFieldKey] = useState<string | null>(null)
  const [dragState, setDragState] = useState<{
    commandIndex: number
    blockIndex: number | null
    position: 'before' | 'after'
  } | null>(null)
  const variableFragments = useMemo(
    () =>
      variables.map((variable) => ({
        ...variable,
        token: `<${variable.key}>`,
      })),
    [variables]
  )

  function updateCommands(
    updater: (prev: AppConfigAgentCommand[]) => AppConfigAgentCommand[]
  ) {
    onChange(updater(commands))
  }

  function updateCommand(
    commandIndex: number,
    updater: (command: AppConfigAgentCommand) => AppConfigAgentCommand
  ) {
    updateCommands((prev) =>
      prev.map((command, index) => (index === commandIndex ? updater(command) : command))
    )
  }

  function updateBlock(
    commandIndex: number,
    blockIndex: number,
    updater: (block: AgentCommandBlock) => AgentCommandBlock
  ) {
    updateCommand(commandIndex, (command) => ({
      ...command,
      blocks: command.blocks.map((block, index) => (index === blockIndex ? updater(block) : block)),
    }))
  }

  function addCommand() {
    updateCommands((prev) => [...prev, buildDefaultAgentCommand(buildNextAgentCommandName(prev))])
  }

  function removeCommand(commandIndex: number) {
    updateCommands((prev) => prev.filter((_, index) => index !== commandIndex))
  }

  function moveCommand(commandIndex: number, direction: -1 | 1) {
    updateCommands((prev) => moveArrayItem(prev, commandIndex, direction))
  }

  function addBlock(commandIndex: number, kind: AgentCommandBlock['kind'] = 'reply_private_message') {
    updateCommand(commandIndex, (command) => ({
      ...command,
      blocks: [...command.blocks, buildDefaultAgentCommandBlock(kind)],
    }))
  }

  function insertBlockAt(
    commandIndex: number,
    insertIndex: number,
    block: AgentCommandBlock
  ) {
    updateCommand(commandIndex, (command) => ({
      ...command,
      blocks: [
        ...command.blocks.slice(0, insertIndex),
        block,
        ...command.blocks.slice(insertIndex),
      ],
    }))
  }

  function removeBlock(commandIndex: number, blockIndex: number) {
    updateCommand(commandIndex, (command) => ({
      ...command,
      blocks: command.blocks.filter((_, index) => index !== blockIndex),
    }))
  }

  function moveBlock(commandIndex: number, blockIndex: number, direction: -1 | 1) {
    updateCommand(commandIndex, (command) => ({
      ...command,
      blocks: moveArrayItem(command.blocks, blockIndex, direction),
    }))
  }

  function setDragPayload(
    event: DragEvent<HTMLElement>,
    payload: AgentBlockDragPayload
  ) {
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.setData(AGENT_BLOCK_DRAG_MIME, JSON.stringify(payload))
  }

  function readDragPayload(event: DragEvent<HTMLElement>): AgentBlockDragPayload | null {
    const raw = event.dataTransfer.getData(AGENT_BLOCK_DRAG_MIME)
    if (!raw) return null
    try {
      return JSON.parse(raw) as AgentBlockDragPayload
    } catch {
      return null
    }
  }

  function performDrop(
    targetCommandIndex: number,
    targetBlockIndex: number | null,
    position: 'before' | 'after'
  ) {
    return (event: DragEvent<HTMLElement>) => {
      event.preventDefault()
      const payload = readDragPayload(event)
      setDragState(null)
      if (!payload) return

      const destinationIndex =
        targetBlockIndex === null
          ? commands[targetCommandIndex]?.blocks.length ?? 0
          : targetBlockIndex + (position === 'after' ? 1 : 0)

      const nextCommands = commands.map((command) => ({
        ...command,
        blocks: [...command.blocks],
      }))

      const targetCommand = nextCommands[targetCommandIndex]
      if (!targetCommand) return

      if ('source' in payload && payload.source === 'variable') {
        if (targetBlockIndex === null) return
        const targetBlock = targetCommand.blocks[targetBlockIndex]
        if (!targetBlock) return
        const fieldKey = activeBlockFieldKey(targetCommandIndex, targetBlockIndex, targetBlock)
        if (!fieldKey) return
        const binding = resolveFieldBinding(targetCommandIndex, targetBlockIndex, targetBlock, fieldKey)
        if (!binding) return
        insertVariableIntoField(fieldKey, binding.value, binding.apply, payload.key)
        return
      }

      if (payload.source === 'palette') {
        targetCommand.blocks.splice(destinationIndex, 0, buildDefaultAgentCommandBlock(payload.option))
        onChange(nextCommands)
        return
      }

      const sourceCommand = nextCommands[payload.commandIndex]
      const draggedBlock = sourceCommand?.blocks[payload.blockIndex]
      if (!sourceCommand || !draggedBlock) return

      const [removedBlock] = sourceCommand.blocks.splice(payload.blockIndex, 1)
      if (!removedBlock) return

      const finalDestinationIndex =
        payload.commandIndex === targetCommandIndex && payload.blockIndex < destinationIndex
          ? destinationIndex - 1
          : destinationIndex

      targetCommand.blocks.splice(finalDestinationIndex, 0, removedBlock)
      onChange(nextCommands)
    }
  }

  function insertVariableTokenIntoBlock(block: AgentCommandBlock, variableKey: string): AgentCommandBlock {
    const token = `<${variableKey}>`
    switch (block.kind) {
      case 'reply_private_message':
      case 'send_webhook':
        return {
          ...block,
          text_template: `${block.text_template}${token}`,
        }
      case 'insert_queued_post':
        return {
          ...block,
          moving_post_code: `${block.moving_post_code}${token}`,
        }
      case 'execute_review_action':
        return {
          ...block,
          review_code: `${block.review_code}${token}`,
          action: insertVariableTokenIntoReviewAction(block.action, token),
        }
      case 'execute_global_action':
        return {
          ...block,
          action: insertVariableTokenIntoGlobalAction(block.action, token),
        }
      default:
        return block
    }
  }

  function insertVariableTokenIntoReviewAction(
    action: AgentCommandReviewAction,
    token: string
  ): AgentCommandReviewAction {
    switch (action.action) {
      case 'defer':
        return { ...action, delay_ms: `${action.delay_ms}${token}` }
      case 'comment':
      case 'reply':
        return { ...action, text_template: `${action.text_template}${token}` }
      case 'blacklist':
        return { ...action, reason_template: `${action.reason_template}${token}` }
      case 'quick_reply':
        return { ...action, key_template: `${action.key_template}${token}` }
      case 'merge':
        return { ...action, target_review_code: `${action.target_review_code}${token}` }
      default:
        return action
    }
  }

  function insertVariableTokenIntoGlobalAction(
    action: AgentCommandGlobalAction,
    token: string
  ): AgentCommandGlobalAction {
    switch (action.action) {
      case 'recall':
      case 'withdraw':
      case 'info':
        return { ...action, review_code: `${action.review_code}${token}` }
      case 'blacklist_add':
        return {
          ...action,
          sender_id: `${action.sender_id}${token}`,
          reason_template: `${action.reason_template}${token}`,
        }
      case 'blacklist_remove':
        return { ...action, sender_id: `${action.sender_id}${token}` }
      case 'set_external_number':
        return { ...action, value_template: `${action.value_template}${token}` }
      case 'quick_reply_add':
        return {
          ...action,
          key_template: `${action.key_template}${token}`,
          text_template: `${action.text_template}${token}`,
        }
      case 'quick_reply_delete':
        return { ...action, key_template: `${action.key_template}${token}` }
      case 'shortcut_add':
        return {
          ...action,
          key_template: `${action.key_template}${token}`,
          definition_template: `${action.definition_template}${token}`,
        }
      case 'shortcut_delete':
        return { ...action, key_template: `${action.key_template}${token}` }
      default:
        return action
    }
  }

  function handleDropZoneEnter(
    commandIndex: number,
    blockIndex: number | null,
    position: 'before' | 'after'
  ) {
    return (event: DragEvent<HTMLElement>) => {
      event.preventDefault()
      setDragState({ commandIndex, blockIndex, position })
    }
  }

  function updateBlockStringList(
    commandIndex: number,
    blockIndex: number,
    field: 'tags' | 'images',
    itemIndex: number,
    value: string
  ) {
    updateRichMessageBlock(commandIndex, blockIndex, (block) => {
      return {
        ...block,
        [field]: block[field].map((item, index) => (index === itemIndex ? value : item)),
      }
    })
  }

  function addBlockStringListItem(
    commandIndex: number,
    blockIndex: number,
    field: 'tags' | 'images'
  ) {
    updateRichMessageBlock(commandIndex, blockIndex, (block) => {
      return {
        ...block,
        [field]: [...block[field], ''],
      }
    })
  }

  function removeBlockStringListItem(
    commandIndex: number,
    blockIndex: number,
    field: 'tags' | 'images',
    itemIndex: number
  ) {
    updateRichMessageBlock(commandIndex, blockIndex, (block) => {
      return {
        ...block,
        [field]: block[field].filter((_, index) => index !== itemIndex),
      }
    })
  }

  function registerFieldRef(key: string, node: AgentCommandFieldNode | null) {
    fieldRefs.current[key] = node
  }

  function insertVariableIntoField(
    fieldKey: string,
    currentValue: string,
    onFieldChange: (nextValue: string) => void,
    variableKey: string
  ) {
    const token = `<${variableKey}>`
    const field = fieldRefs.current[fieldKey]
    const selectionStart = field?.selectionStart ?? currentValue.length
    const selectionEnd = field?.selectionEnd ?? currentValue.length
    const nextValue = `${currentValue.slice(0, selectionStart)}${token}${currentValue.slice(selectionEnd)}`
    onFieldChange(nextValue)
    window.requestAnimationFrame(() => {
      const nextField = fieldRefs.current[fieldKey]
      const nextCaret = selectionStart + token.length
      nextField?.focus()
      nextField?.setSelectionRange(nextCaret, nextCaret)
      setActiveFieldKey(fieldKey)
    })
  }

  function bindVariableDropHandlers(
    commandIndex: number,
    blockIndex: number,
    block: AgentCommandBlock,
    fieldKey: string,
    currentValue: string,
    onFieldChange: (nextValue: string) => void
  ) {
    return {
      onDragOver: (event: DragEvent<HTMLElement>) => {
        const payload = readDragPayload(event)
        if (payload && 'source' in payload && payload.source === 'variable') {
          event.preventDefault()
        }
      },
      onDrop: (event: DragEvent<HTMLElement>) => {
        const payload = readDragPayload(event)
        if (!payload || !('source' in payload) || payload.source !== 'variable') return
        event.preventDefault()
        insertVariableIntoField(fieldKey, currentValue, onFieldChange, payload.key)
        setActiveFieldKey(fieldKey)
        void commandIndex
        void blockIndex
        void block
      },
    }
  }

  function resolveFieldBindingByKey(fieldKey: string): {
    value: string
    apply: (nextValue: string) => void
  } | null {
    const match = /^agent-(\d+)-(\d+)-(.+)$/.exec(fieldKey)
    if (!match) return null

    const commandIndex = Number(match[1])
    const blockIndex = Number(match[2])
    const command = commands[commandIndex]
    const block = command?.blocks[blockIndex]
    if (!command || !block) return null

    return resolveFieldBinding(commandIndex, blockIndex, block, fieldKey)
  }

  function getDefaultVariableTargetFieldKey() {
    for (let commandIndex = 0; commandIndex < commands.length; commandIndex += 1) {
      const command = commands[commandIndex]
      for (let blockIndex = 0; blockIndex < command.blocks.length; blockIndex += 1) {
        const block = command.blocks[blockIndex]
        const fieldKey = buildDefaultBlockFieldKey(commandIndex, blockIndex, block)
        if (fieldKey) return fieldKey
      }
    }
    return null
  }

  function pickVariableFragment(variableKey: string) {
    const activeTargetKey =
      activeFieldKey && resolveFieldBindingByKey(activeFieldKey) ? activeFieldKey : null
    const targetFieldKey = activeTargetKey ?? getDefaultVariableTargetFieldKey()
    if (!targetFieldKey) return

    const binding = resolveFieldBindingByKey(targetFieldKey)
    if (!binding) return

    insertVariableIntoField(targetFieldKey, binding.value, binding.apply, variableKey)
  }

  function changeBlockKind(commandIndex: number, blockIndex: number, value: string) {
    setActiveFieldKey(null)
    updateBlock(commandIndex, blockIndex, (block) =>
      convertAgentCommandBlockKind(block, value as AgentCommandBlockOptionValue)
    )
  }

  function updateRichMessageBlock(
    commandIndex: number,
    blockIndex: number,
    updater: (block: ReplyPrivateMessageBlock | SendWebhookBlock) => ReplyPrivateMessageBlock | SendWebhookBlock
  ) {
    updateBlock(commandIndex, blockIndex, (block) => {
      if (block.kind !== 'reply_private_message' && block.kind !== 'send_webhook') return block
      return updater(block)
    })
  }

  function updateReviewActionBlock(
    commandIndex: number,
    blockIndex: number,
    updater: (block: ExecuteReviewActionBlock) => ExecuteReviewActionBlock
  ) {
    updateBlock(commandIndex, blockIndex, (block) => {
      if (block.kind !== 'execute_review_action') return block
      return updater(block)
    })
  }

  function updateReviewAction(
    commandIndex: number,
    blockIndex: number,
    updater: (action: AgentCommandReviewAction) => AgentCommandReviewAction
  ) {
    updateReviewActionBlock(commandIndex, blockIndex, (block) => ({
      ...block,
      action: updater(block.action),
    }))
  }

  function updateGlobalActionBlock(
    commandIndex: number,
    blockIndex: number,
    updater: (block: ExecuteGlobalActionBlock) => ExecuteGlobalActionBlock
  ) {
    updateBlock(commandIndex, blockIndex, (block) => {
      if (block.kind !== 'execute_global_action') return block
      return updater(block)
    })
  }

  function updateGlobalAction(
    commandIndex: number,
    blockIndex: number,
    updater: (action: AgentCommandGlobalAction) => AgentCommandGlobalAction
  ) {
    updateGlobalActionBlock(commandIndex, blockIndex, (block) => ({
      ...block,
      action: updater(block.action),
    }))
  }

  function parseIndexedFieldName(fieldName: string, prefix: string) {
    if (!fieldName.startsWith(prefix)) return null
    const index = Number(fieldName.slice(prefix.length))
    if (!Number.isInteger(index) || index < 0) return null
    return index
  }

  function resolveFieldBinding(
    commandIndex: number,
    blockIndex: number,
    block: AgentCommandBlock,
    fieldKey: string
  ): { value: string; apply: (nextValue: string) => void } | null {
    const prefix = `agent-${commandIndex}-${blockIndex}-`
    if (!fieldKey.startsWith(prefix)) return null
    const fieldName = fieldKey.slice(prefix.length)

    switch (block.kind) {
      case 'reply_private_message':
      case 'send_webhook':
        if (fieldName === 'text_template') {
          return {
            value: block.text_template,
            apply: (nextValue) =>
              updateRichMessageBlock(commandIndex, blockIndex, (current) => ({
                ...current,
                text_template: nextValue,
              })),
          }
        }
        if (block.kind === 'send_webhook' && fieldName === 'url') {
          return {
            value: block.url,
            apply: (nextValue) =>
              updateRichMessageBlock(commandIndex, blockIndex, (current) => {
                if (current.kind !== 'send_webhook') return current
                return { ...current, url: nextValue }
              }),
          }
        }
        if (block.kind === 'send_webhook' && fieldName === 'source_webhook') {
          return {
            value: block.source_webhook,
            apply: (nextValue) =>
              updateRichMessageBlock(commandIndex, blockIndex, (current) => {
                if (current.kind !== 'send_webhook') return current
                return { ...current, source_webhook: nextValue }
              }),
          }
        }
        {
          const tagIndex = parseIndexedFieldName(fieldName, 'tag-')
          if (tagIndex !== null) {
            return {
              value: block.tags[tagIndex] ?? '',
              apply: (nextValue) => updateBlockStringList(commandIndex, blockIndex, 'tags', tagIndex, nextValue),
            }
          }
        }
        {
          const imageIndex = parseIndexedFieldName(fieldName, 'image-')
          if (imageIndex !== null) {
            return {
              value: block.images[imageIndex] ?? '',
              apply: (nextValue) => updateBlockStringList(commandIndex, blockIndex, 'images', imageIndex, nextValue),
            }
          }
        }
        return null
      case 'insert_queued_post':
        if (fieldName === 'moving_post_code') {
          return {
            value: block.moving_post_code,
            apply: (nextValue) =>
              updateBlock(commandIndex, blockIndex, (current) => {
                if (current.kind !== 'insert_queued_post') return current
                return { ...current, moving_post_code: nextValue }
              }),
          }
        }
        if (fieldName === 'anchor_post_code') {
          return {
            value: block.anchor_post_code,
            apply: (nextValue) =>
              updateBlock(commandIndex, blockIndex, (current) => {
                if (current.kind !== 'insert_queued_post') return current
                return { ...current, anchor_post_code: nextValue }
              }),
          }
        }
        return null
      case 'execute_review_action':
        if (fieldName === 'review_code') {
          return {
            value: block.review_code,
            apply: (nextValue) =>
              updateReviewActionBlock(commandIndex, blockIndex, (current) => ({
                ...current,
                review_code: nextValue,
              })),
          }
        }
        switch (block.action.action) {
          case 'defer':
            if (fieldName === 'action.delay_ms') {
              return {
                value: block.action.delay_ms,
                apply: (nextValue) =>
                  updateReviewAction(commandIndex, blockIndex, (action) =>
                    action.action === 'defer' ? { ...action, delay_ms: nextValue } : action
                  ),
              }
            }
            return null
          case 'comment':
          case 'reply':
            if (fieldName === 'action.text_template') {
              return {
                value: block.action.text_template,
                apply: (nextValue) =>
                  updateReviewAction(commandIndex, blockIndex, (action) =>
                    action.action === 'comment' || action.action === 'reply'
                      ? { ...action, text_template: nextValue }
                      : action
                  ),
              }
            }
            return null
          case 'blacklist':
            if (fieldName === 'action.reason_template') {
              return {
                value: block.action.reason_template,
                apply: (nextValue) =>
                  updateReviewAction(commandIndex, blockIndex, (action) =>
                    action.action === 'blacklist' ? { ...action, reason_template: nextValue } : action
                  ),
              }
            }
            return null
          case 'quick_reply':
            if (fieldName === 'action.key_template') {
              return {
                value: block.action.key_template,
                apply: (nextValue) =>
                  updateReviewAction(commandIndex, blockIndex, (action) =>
                    action.action === 'quick_reply' ? { ...action, key_template: nextValue } : action
                  ),
              }
            }
            return null
          case 'merge':
            if (fieldName === 'action.target_review_code') {
              return {
                value: block.action.target_review_code,
                apply: (nextValue) =>
                  updateReviewAction(commandIndex, blockIndex, (action) =>
                    action.action === 'merge' ? { ...action, target_review_code: nextValue } : action
                  ),
              }
            }
            return null
          default:
            return null
        }
      case 'execute_global_action':
        switch (block.action.action) {
          case 'recall':
          case 'withdraw':
          case 'info':
            if (fieldName === 'action.review_code') {
              return {
                value: block.action.review_code,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'recall' || action.action === 'withdraw' || action.action === 'info'
                      ? { ...action, review_code: nextValue }
                      : action
                  ),
              }
            }
            return null
          case 'blacklist_add':
            if (fieldName === 'action.sender_id') {
              return {
                value: block.action.sender_id,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'blacklist_add' ? { ...action, sender_id: nextValue } : action
                  ),
              }
            }
            if (fieldName === 'action.reason_template') {
              return {
                value: block.action.reason_template,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'blacklist_add' ? { ...action, reason_template: nextValue } : action
                  ),
              }
            }
            return null
          case 'blacklist_remove':
            if (fieldName === 'action.sender_id') {
              return {
                value: block.action.sender_id,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'blacklist_remove' ? { ...action, sender_id: nextValue } : action
                  ),
              }
            }
            return null
          case 'set_external_number':
            if (fieldName === 'action.value_template') {
              return {
                value: block.action.value_template,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'set_external_number' ? { ...action, value_template: nextValue } : action
                  ),
              }
            }
            return null
          case 'quick_reply_add':
            if (fieldName === 'action.key_template') {
              return {
                value: block.action.key_template,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'quick_reply_add' ? { ...action, key_template: nextValue } : action
                  ),
              }
            }
            if (fieldName === 'action.text_template') {
              return {
                value: block.action.text_template,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'quick_reply_add' ? { ...action, text_template: nextValue } : action
                  ),
              }
            }
            return null
          case 'quick_reply_delete':
            if (fieldName === 'action.key_template') {
              return {
                value: block.action.key_template,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'quick_reply_delete' ? { ...action, key_template: nextValue } : action
                  ),
              }
            }
            return null
          case 'shortcut_add':
            if (fieldName === 'action.key_template') {
              return {
                value: block.action.key_template,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_add' ? { ...action, key_template: nextValue } : action
                  ),
              }
            }
            if (fieldName === 'action.definition_template') {
              return {
                value: block.action.definition_template,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_add'
                      ? { ...action, definition_template: nextValue }
                      : action
                  ),
              }
            }
            return null
          case 'shortcut_delete':
            if (fieldName === 'action.key_template') {
              return {
                value: block.action.key_template,
                apply: (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_delete' ? { ...action, key_template: nextValue } : action
                  ),
              }
            }
            return null
          default:
            return null
        }
      default:
        return null
    }
  }

  function buildDefaultBlockFieldKey(
    commandIndex: number,
    blockIndex: number,
    block: AgentCommandBlock
  ): string | null {
    switch (block.kind) {
      case 'reply_private_message':
      case 'send_webhook':
        return `agent-${commandIndex}-${blockIndex}-text_template`
      case 'insert_queued_post':
        return `agent-${commandIndex}-${blockIndex}-moving_post_code`
      case 'execute_review_action':
        switch (block.action.action) {
          case 'defer':
            return `agent-${commandIndex}-${blockIndex}-action.delay_ms`
          case 'comment':
          case 'reply':
            return `agent-${commandIndex}-${blockIndex}-action.text_template`
          case 'blacklist':
            return `agent-${commandIndex}-${blockIndex}-action.reason_template`
          case 'quick_reply':
            return `agent-${commandIndex}-${blockIndex}-action.key_template`
          case 'merge':
            return `agent-${commandIndex}-${blockIndex}-action.target_review_code`
          default:
            return `agent-${commandIndex}-${blockIndex}-review_code`
        }
      case 'execute_global_action':
        switch (block.action.action) {
          case 'recall':
          case 'withdraw':
          case 'info':
            return `agent-${commandIndex}-${blockIndex}-action.review_code`
          case 'blacklist_add':
          case 'blacklist_remove':
            return `agent-${commandIndex}-${blockIndex}-action.sender_id`
          case 'set_external_number':
            return `agent-${commandIndex}-${blockIndex}-action.value_template`
          case 'quick_reply_add':
            return `agent-${commandIndex}-${blockIndex}-action.key_template`
          case 'quick_reply_delete':
            return `agent-${commandIndex}-${blockIndex}-action.key_template`
          case 'shortcut_add':
            return `agent-${commandIndex}-${blockIndex}-action.key_template`
          case 'shortcut_delete':
            return `agent-${commandIndex}-${blockIndex}-action.key_template`
          default:
            return null
        }
      default:
        return null
    }
  }

  function activeBlockFieldKey(
    commandIndex: number,
    blockIndex: number,
    block: AgentCommandBlock
  ) {
    const prefix = `agent-${commandIndex}-${blockIndex}-`
    if (activeFieldKey?.startsWith(prefix)) return activeFieldKey
    return buildDefaultBlockFieldKey(commandIndex, blockIndex, block)
  }

  function renderReviewActionFields(
    commandIndex: number,
    blockIndex: number,
    block: ExecuteReviewActionBlock
  ) {
    const actionKey = block.action.action as AgentReviewActionKey
    return (
      <div className="agent-command-block-grid">
        <div
          className="field-stack fragment-drop-target"
          {...bindVariableDropHandlers(
            commandIndex,
            blockIndex,
            block,
            `agent-${commandIndex}-${blockIndex}-review_code`,
            block.review_code,
            (nextValue) =>
              updateReviewActionBlock(commandIndex, blockIndex, (current) => ({
                ...current,
                review_code: nextValue,
              }))
          )}
        >
          <span className="field-label">审核目标编号</span>
          <Input
            ref={(node) => registerFieldRef(`agent-${commandIndex}-${blockIndex}-review_code`, node)}
            placeholder="例如 12345 或 <previous_post_internal_code>"
            value={block.review_code}
            onFocus={() => setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-review_code`)}
            onChange={(event) =>
              updateReviewActionBlock(commandIndex, blockIndex, (current) => ({
                ...current,
                review_code: event.target.value,
              }))
            }
          />
        </div>
        <div className="field-stack">
          <span className="field-label">审核动作</span>
          <HeroSelect
            ariaLabel={`选择审核动作 ${commandIndex + 1}-${blockIndex + 1}`}
            selectedKey={actionKey}
            options={AGENT_REVIEW_ACTION_OPTIONS}
            onSelect={(value) =>
              updateReviewActionBlock(commandIndex, blockIndex, (current) => ({
                ...current,
                action: convertAgentCommandReviewAction(current.action, value as AgentReviewActionKey),
              }))
            }
          />
        </div>

        {block.action.action === 'defer' && (
          <div
            className="field-stack agent-command-block-span-2 fragment-drop-target"
            {...bindVariableDropHandlers(
              commandIndex,
              blockIndex,
              block,
              `agent-${commandIndex}-${blockIndex}-action.delay_ms`,
              block.action.delay_ms,
              (nextValue) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'defer' ? { ...action, delay_ms: nextValue } : action
                )
            )}
          >
            <span className="field-label">延后毫秒数</span>
            <Input
              ref={(node) =>
                registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.delay_ms`, node)
              }
              placeholder="例如 180000 或 <received_timestamp_ms>"
              value={block.action.delay_ms}
              onFocus={() =>
                setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.delay_ms`)
              }
              onChange={(event) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'defer' ? { ...action, delay_ms: event.target.value } : action
                )
              }
            />
          </div>
        )}

        {(block.action.action === 'comment' || block.action.action === 'reply') && (
          <div
            className="field-stack agent-command-block-span-2 fragment-drop-target"
            {...bindVariableDropHandlers(
              commandIndex,
              blockIndex,
              block,
              `agent-${commandIndex}-${blockIndex}-action.text_template`,
              block.action.text_template,
              (nextValue) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'comment' || action.action === 'reply'
                    ? { ...action, text_template: nextValue }
                    : action
                )
            )}
          >
            <span className="field-label">
              {block.action.action === 'comment' ? '评论文案' : '回复文案'}
            </span>
            <TextArea
              ref={(node) =>
                registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.text_template`, node)
              }
              placeholder="支持使用 <变量>"
              value={block.action.text_template}
              onFocus={() =>
                setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.text_template`)
              }
              onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'comment' || action.action === 'reply'
                    ? { ...action, text_template: event.target.value }
                    : action
                )
              }
            />
          </div>
        )}

        {block.action.action === 'blacklist' && (
          <div
            className="field-stack agent-command-block-span-2 fragment-drop-target"
            {...bindVariableDropHandlers(
              commandIndex,
              blockIndex,
              block,
              `agent-${commandIndex}-${blockIndex}-action.reason_template`,
              block.action.reason_template,
              (nextValue) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'blacklist'
                    ? { ...action, reason_template: nextValue }
                    : action
                )
            )}
          >
            <span className="field-label">拉黑原因</span>
            <TextArea
              ref={(node) =>
                registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.reason_template`, node)
              }
              placeholder="可留空，支持使用 <变量>"
              value={block.action.reason_template}
              onFocus={() =>
                setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.reason_template`)
              }
              onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'blacklist'
                    ? { ...action, reason_template: event.target.value }
                    : action
                )
              }
            />
          </div>
        )}

        {block.action.action === 'quick_reply' && (
          <div
            className="field-stack agent-command-block-span-2 fragment-drop-target"
            {...bindVariableDropHandlers(
              commandIndex,
              blockIndex,
              block,
              `agent-${commandIndex}-${blockIndex}-action.key_template`,
              block.action.key_template,
              (nextValue) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'quick_reply'
                    ? { ...action, key_template: nextValue }
                    : action
                )
            )}
          >
            <span className="field-label">快捷回复键名</span>
            <Input
              ref={(node) =>
                registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.key_template`, node)
              }
              placeholder="例如 default 或 <command_args>"
              value={block.action.key_template}
              onFocus={() =>
                setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.key_template`)
              }
              onChange={(event) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'quick_reply'
                    ? { ...action, key_template: event.target.value }
                    : action
                )
              }
            />
          </div>
        )}

        {block.action.action === 'merge' && (
          <div
            className="field-stack agent-command-block-span-2 fragment-drop-target"
            {...bindVariableDropHandlers(
              commandIndex,
              blockIndex,
              block,
              `agent-${commandIndex}-${blockIndex}-action.target_review_code`,
              block.action.target_review_code,
              (nextValue) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'merge'
                    ? { ...action, target_review_code: nextValue }
                    : action
                )
            )}
          >
            <span className="field-label">目标审核编号</span>
            <Input
              ref={(node) =>
                registerFieldRef(
                  `agent-${commandIndex}-${blockIndex}-action.target_review_code`,
                  node
                )
              }
              placeholder="例如 12345"
              value={block.action.target_review_code}
              onFocus={() =>
                setActiveFieldKey(
                  `agent-${commandIndex}-${blockIndex}-action.target_review_code`
                )
              }
              onChange={(event) =>
                updateReviewAction(commandIndex, blockIndex, (action) =>
                  action.action === 'merge'
                    ? { ...action, target_review_code: event.target.value }
                    : action
                )
              }
            />
          </div>
        )}

        {(['approve', 'reject', 'delete', 'skip', 'immediate', 'refresh', 'rerender', 'select_all_messages', 'toggle_anonymous', 'expand_audit', 'show'] as AgentReviewActionKey[]).includes(actionKey) && (
          <p className="field-hint agent-command-block-span-2">
            当前审核动作不需要额外参数，执行时会直接作用到上方填写的审核编号。
          </p>
        )}
      </div>
    )
  }

  function renderGlobalActionFields(
    commandIndex: number,
    blockIndex: number,
    block: ExecuteGlobalActionBlock
  ) {
    const actionKey = block.action.action as AgentGlobalActionKey
    return (
      <div className="agent-command-block-grid">
        <div className="field-stack agent-command-block-span-2">
          <span className="field-label">系统动作</span>
          <HeroSelect
            ariaLabel={`选择系统动作 ${commandIndex + 1}-${blockIndex + 1}`}
            selectedKey={actionKey}
            options={AGENT_GLOBAL_ACTION_OPTIONS}
            onSelect={(value) =>
              updateGlobalActionBlock(commandIndex, blockIndex, (current) => ({
                ...current,
                action: convertAgentCommandGlobalAction(current.action, value as AgentGlobalActionKey),
              }))
            }
          />
        </div>

        {(block.action.action === 'recall' ||
          block.action.action === 'withdraw' ||
          block.action.action === 'info') && (
          <div
            className="field-stack agent-command-block-span-2 fragment-drop-target"
            {...bindVariableDropHandlers(
              commandIndex,
              blockIndex,
              block,
              `agent-${commandIndex}-${blockIndex}-action.review_code`,
              block.action.review_code,
              (nextValue) =>
                updateGlobalAction(commandIndex, blockIndex, (action) =>
                  action.action === 'recall' ||
                  action.action === 'withdraw' ||
                  action.action === 'info'
                    ? { ...action, review_code: nextValue }
                    : action
                )
            )}
          >
            <span className="field-label">审核编号</span>
            <Input
              ref={(node) =>
                registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.review_code`, node)
              }
              placeholder="例如 12345 或 <previous_post_internal_code>"
              value={block.action.review_code}
              onFocus={() =>
                setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.review_code`)
              }
              onChange={(event) =>
                updateGlobalAction(commandIndex, blockIndex, (action) =>
                  action.action === 'recall' ||
                  action.action === 'withdraw' ||
                  action.action === 'info'
                    ? { ...action, review_code: event.target.value }
                    : action
                )
              }
            />
          </div>
        )}

        {block.action.action === 'blacklist_add' && (
          <>
            <div
              className="field-stack fragment-drop-target"
              {...bindVariableDropHandlers(
                commandIndex,
                blockIndex,
                block,
                `agent-${commandIndex}-${blockIndex}-action.sender_id`,
                block.action.sender_id,
                (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'blacklist_add'
                      ? { ...action, sender_id: nextValue }
                      : action
                  )
              )}
            >
              <span className="field-label">投稿人 QQ</span>
              <Input
                ref={(node) =>
                  registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.sender_id`, node)
                }
                placeholder="例如 <sender_id>"
                value={block.action.sender_id}
                onFocus={() =>
                  setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.sender_id`)
                }
                onChange={(event) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'blacklist_add'
                      ? { ...action, sender_id: event.target.value }
                      : action
                  )
                }
              />
            </div>
            <div className="field-stack">
              <span className="field-label">拉黑原因</span>
              <TextArea
                ref={(node) =>
                  registerFieldRef(
                    `agent-${commandIndex}-${blockIndex}-action.reason_template`,
                    node
                  )
                }
                placeholder="可留空，支持使用 <变量>"
                value={block.action.reason_template}
                onFocus={() =>
                  setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.reason_template`)
                }
                onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'blacklist_add'
                      ? { ...action, reason_template: event.target.value }
                      : action
                  )
                }
              />
            </div>
          </>
        )}

        {block.action.action === 'blacklist_remove' && (
          <div
            className="field-stack agent-command-block-span-2 fragment-drop-target"
            {...bindVariableDropHandlers(
              commandIndex,
              blockIndex,
              block,
              `agent-${commandIndex}-${blockIndex}-action.sender_id`,
              block.action.sender_id,
              (nextValue) =>
                updateGlobalAction(commandIndex, blockIndex, (action) =>
                  action.action === 'blacklist_remove'
                    ? { ...action, sender_id: nextValue }
                    : action
                )
            )}
          >
            <span className="field-label">投稿人 QQ</span>
            <Input
              ref={(node) =>
                registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.sender_id`, node)
              }
              placeholder="例如 <sender_id>"
              value={block.action.sender_id}
              onFocus={() =>
                setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.sender_id`)
              }
              onChange={(event) =>
                updateGlobalAction(commandIndex, blockIndex, (action) =>
                  action.action === 'blacklist_remove'
                    ? { ...action, sender_id: event.target.value }
                    : action
                )
              }
            />
          </div>
        )}

        {block.action.action === 'set_external_number' && (
          <div
            className="field-stack agent-command-block-span-2 fragment-drop-target"
            {...bindVariableDropHandlers(
              commandIndex,
              blockIndex,
              block,
              `agent-${commandIndex}-${blockIndex}-action.value_template`,
              block.action.value_template,
              (nextValue) =>
                updateGlobalAction(commandIndex, blockIndex, (action) =>
                  action.action === 'set_external_number'
                    ? { ...action, value_template: nextValue }
                    : action
                )
            )}
          >
            <span className="field-label">外部编号</span>
            <Input
              ref={(node) =>
                registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.value_template`, node)
              }
              placeholder="例如 3344846508"
              value={block.action.value_template}
              onFocus={() =>
                setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.value_template`)
              }
              onChange={(event) =>
                updateGlobalAction(commandIndex, blockIndex, (action) =>
                  action.action === 'set_external_number'
                    ? { ...action, value_template: event.target.value }
                    : action
                )
              }
            />
          </div>
        )}

        {block.action.action === 'quick_reply_add' && (
          <>
            <div
              className="field-stack fragment-drop-target"
              {...bindVariableDropHandlers(
                commandIndex,
                blockIndex,
                block,
                `agent-${commandIndex}-${blockIndex}-action.key_template`,
                block.action.key_template,
                (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'quick_reply_add'
                      ? { ...action, key_template: nextValue }
                      : action
                  )
              )}
            >
              <span className="field-label">快捷回复键名</span>
              <Input
                ref={(node) =>
                  registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.key_template`, node)
                }
                placeholder="例如 help"
                value={block.action.key_template}
                onFocus={() =>
                  setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.key_template`)
                }
                onChange={(event) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'quick_reply_add'
                      ? { ...action, key_template: event.target.value }
                      : action
                  )
                }
              />
            </div>
            <div
              className="field-stack fragment-drop-target"
              {...bindVariableDropHandlers(
                commandIndex,
                blockIndex,
                block,
                `agent-${commandIndex}-${blockIndex}-action.text_template`,
                block.action.text_template,
                (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'quick_reply_add'
                      ? { ...action, text_template: nextValue }
                      : action
                  )
              )}
            >
              <span className="field-label">快捷回复内容</span>
              <TextArea
                ref={(node) =>
                  registerFieldRef(
                    `agent-${commandIndex}-${blockIndex}-action.text_template`,
                    node
                  )
                }
                placeholder="支持使用 <变量>"
                value={block.action.text_template}
                onFocus={() =>
                  setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.text_template`)
                }
                onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'quick_reply_add'
                      ? { ...action, text_template: event.target.value }
                      : action
                  )
                }
              />
            </div>
          </>
        )}

        {block.action.action === 'quick_reply_delete' && (
          <div
            className="field-stack agent-command-block-span-2 fragment-drop-target"
            {...bindVariableDropHandlers(
              commandIndex,
              blockIndex,
              block,
              `agent-${commandIndex}-${blockIndex}-action.key_template`,
              block.action.key_template,
              (nextValue) =>
                updateGlobalAction(commandIndex, blockIndex, (action) =>
                  action.action === 'quick_reply_delete'
                    ? { ...action, key_template: nextValue }
                    : action
                )
            )}
          >
            <span className="field-label">快捷回复键名</span>
            <Input
              ref={(node) =>
                registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.key_template`, node)
              }
              placeholder="例如 help"
              value={block.action.key_template}
              onFocus={() =>
                setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.key_template`)
              }
              onChange={(event) =>
                updateGlobalAction(commandIndex, blockIndex, (action) =>
                  action.action === 'quick_reply_delete'
                    ? { ...action, key_template: event.target.value }
                    : action
                )
              }
            />
          </div>
        )}

        {block.action.action === 'shortcut_add' && (
          <>
            <div className="field-stack">
              <span className="field-label">快捷指令作用域</span>
              <HeroSelect
                ariaLabel={`选择快捷指令作用域 ${commandIndex + 1}-${blockIndex + 1}`}
                selectedKey={block.action.scope}
                options={AGENT_SHORTCUT_SCOPE_OPTIONS}
                onSelect={(value) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_add'
                      ? { ...action, scope: value as AgentCommandShortcutScope }
                      : action
                  )
                }
              />
            </div>
            <div
              className="field-stack fragment-drop-target"
              {...bindVariableDropHandlers(
                commandIndex,
                blockIndex,
                block,
                `agent-${commandIndex}-${blockIndex}-action.key_template`,
                block.action.key_template,
                (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_add'
                      ? { ...action, key_template: nextValue }
                      : action
                  )
              )}
            >
              <span className="field-label">快捷指令键名</span>
              <Input
                ref={(node) =>
                  registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.key_template`, node)
                }
                placeholder="例如 h"
                value={block.action.key_template}
                onFocus={() =>
                  setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.key_template`)
                }
                onChange={(event) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_add'
                      ? { ...action, key_template: event.target.value }
                      : action
                  )
                }
              />
            </div>
            <div
              className="field-stack agent-command-block-span-2 fragment-drop-target"
              {...bindVariableDropHandlers(
                commandIndex,
                blockIndex,
                block,
                `agent-${commandIndex}-${blockIndex}-action.definition_template`,
                block.action.definition_template,
                (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_add'
                      ? { ...action, definition_template: nextValue }
                      : action
                  )
              )}
            >
              <span className="field-label">快捷指令定义</span>
              <TextArea
                ref={(node) =>
                  registerFieldRef(
                    `agent-${commandIndex}-${blockIndex}-action.definition_template`,
                    node
                  )
                }
                placeholder="例如 匿| 是"
                value={block.action.definition_template}
                onFocus={() =>
                  setActiveFieldKey(
                    `agent-${commandIndex}-${blockIndex}-action.definition_template`
                  )
                }
                onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_add'
                      ? { ...action, definition_template: event.target.value }
                      : action
                  )
                }
              />
            </div>
          </>
        )}

        {block.action.action === 'shortcut_delete' && (
          <>
            <div className="field-stack">
              <span className="field-label">快捷指令作用域</span>
              <HeroSelect
                ariaLabel={`选择删除快捷指令作用域 ${commandIndex + 1}-${blockIndex + 1}`}
                selectedKey={block.action.scope}
                options={AGENT_SHORTCUT_SCOPE_OPTIONS}
                onSelect={(value) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_delete'
                      ? { ...action, scope: value as AgentCommandShortcutScope }
                      : action
                  )
                }
              />
            </div>
            <div
              className="field-stack fragment-drop-target"
              {...bindVariableDropHandlers(
                commandIndex,
                blockIndex,
                block,
                `agent-${commandIndex}-${blockIndex}-action.key_template`,
                block.action.key_template,
                (nextValue) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_delete'
                      ? { ...action, key_template: nextValue }
                      : action
                  )
              )}
            >
              <span className="field-label">快捷指令键名</span>
              <Input
                ref={(node) =>
                  registerFieldRef(`agent-${commandIndex}-${blockIndex}-action.key_template`, node)
                }
                placeholder="例如 h"
                value={block.action.key_template}
                onFocus={() =>
                  setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-action.key_template`)
                }
                onChange={(event) =>
                  updateGlobalAction(commandIndex, blockIndex, (action) =>
                    action.action === 'shortcut_delete'
                      ? { ...action, key_template: event.target.value }
                      : action
                  )
                }
              />
            </div>
          </>
        )}

        {(['help', 'manual_relogin', 'auto_relogin', 'pending_list', 'pending_clear', 'send_queue_clear', 'send_queue_flush', 'send_in_flight_clear', 'blacklist_list', 'quick_reply_list', 'shortcut_list', 'self_check', 'system_repair'] as AgentGlobalActionKey[]).includes(actionKey) && (
          <p className="field-hint agent-command-block-span-2">
            当前系统动作不需要额外参数，执行时会直接调用对应的后端能力。
          </p>
        )}
      </div>
    )
  }

  function buildAgentCommandBlockSelectValue(block: AgentCommandBlock): AgentCommandBlockOptionValue {
    switch (block.kind) {
      case 'execute_review_action':
        return `review:${block.action.action}`
      case 'execute_global_action':
        return `global:${block.action.action}`
      default:
        return block.kind
    }
  }

  const paletteGroups = [
    {
      title: '消息与会话',
      options: AGENT_COMMAND_BLOCK_OPTIONS.filter((option) =>
        [
          'reply_private_message',
          'start_submission_session',
          'finish_submission_session',
          'resume_submission_session',
          'submit_submission_session',
          'cancel_submission_session',
          'send_webhook',
        ].includes(option.value)
      ),
    },
    {
      title: '审核积木',
      options: AGENT_COMMAND_BLOCK_OPTIONS.filter((option) => option.value.startsWith('review:')),
    },
    {
      title: '系统积木',
      options: AGENT_COMMAND_BLOCK_OPTIONS.filter(
        (option) => option.value === 'insert_queued_post' || option.value.startsWith('global:')
      ),
    },
  ]

  function renderCommandBlock(
    command: AppConfigAgentCommand,
    commandIndex: number,
    block: AgentCommandBlock,
    blockIndex: number
  ) {
    const blockKeyPrefix = `agent-${commandIndex}-${blockIndex}`
    const isBeforeDropActive =
      dragState?.commandIndex === commandIndex &&
      dragState?.blockIndex === blockIndex &&
      dragState?.position === 'before'
    const isAfterDropActive =
      dragState?.commandIndex === commandIndex &&
      dragState?.blockIndex === blockIndex &&
      dragState?.position === 'after'

    return (
      <div key={`${blockKeyPrefix}-wrap`}>
        <div
          className={`agent-block-dropzone ${isBeforeDropActive ? 'is-active' : ''}`}
          onDragOver={(event) => event.preventDefault()}
          onDragEnter={handleDropZoneEnter(commandIndex, blockIndex, 'before')}
          onDrop={performDrop(commandIndex, blockIndex, 'before')}
        />
        <div
          className="agent-command-block"
          draggable
          onDragOver={(event) => {
            const payload = readDragPayload(event)
            if (payload && 'source' in payload && payload.source === 'variable') {
              event.preventDefault()
            }
          }}
          onDrop={(event) => {
            const payload = readDragPayload(event)
            if (!payload || !('source' in payload) || payload.source !== 'variable') return
            event.preventDefault()
            updateBlock(commandIndex, blockIndex, (current) =>
              insertVariableTokenIntoBlock(current, payload.key)
            )
          }}
          onDragStart={(event) =>
            setDragPayload(event, {
              source: 'workspace',
              commandIndex,
              blockIndex,
            })
          }
          onDragEnd={() => setDragState(null)}
        >
          <div className="agent-command-block-head">
            <div className="field-stack">
              <span className="field-label">积木 {blockIndex + 1}</span>
              <HeroSelect
                ariaLabel={`选择指令 ${commandIndex + 1} 的积木 ${blockIndex + 1}`}
                selectedKey={buildAgentCommandBlockSelectValue(block)}
                options={AGENT_COMMAND_BLOCK_OPTIONS}
                onSelect={(value) => changeBlockKind(commandIndex, blockIndex, value)}
              />
            </div>
            <div className="agent-command-actions">
              <span className="agent-drag-hint">拖拽排序</span>
              <Button
                size="sm"
                variant="secondary"
                isDisabled={blockIndex === 0}
                onClick={() => moveBlock(commandIndex, blockIndex, -1)}
              >
                上移
              </Button>
              <Button
                size="sm"
                variant="secondary"
                isDisabled={blockIndex >= command.blocks.length - 1}
                onClick={() => moveBlock(commandIndex, blockIndex, 1)}
              >
                下移
              </Button>
              <Button
                size="sm"
                variant="tertiary"
                onClick={() => removeBlock(commandIndex, blockIndex)}
              >
                删除积木
              </Button>
            </div>
          </div>

          {block.kind === 'reply_private_message' && (
            <div className="agent-command-block-grid">
              <div
                className="field-stack agent-command-block-span-2 fragment-drop-target"
                {...bindVariableDropHandlers(
                  commandIndex,
                  blockIndex,
                  block,
                  `${blockKeyPrefix}-text_template`,
                  block.text_template,
                  (nextValue) =>
                    updateRichMessageBlock(commandIndex, blockIndex, (current) => ({
                      ...current,
                      text_template: nextValue,
                    }))
                )}
              >
                <span className="field-label">回复文案</span>
                <TextArea
                  ref={(node) => registerFieldRef(`${blockKeyPrefix}-text_template`, node)}
                  value={block.text_template}
                  onFocus={() => setActiveFieldKey(`${blockKeyPrefix}-text_template`)}
                  onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                    updateRichMessageBlock(commandIndex, blockIndex, (current) => ({
                      ...current,
                      text_template: event.target.value,
                    }))
                  }
                />
              </div>
              <AgentCommandStringListEditor
                label="额外标签"
                placeholder="例如 help 或 sender_id"
                values={block.tags}
                fieldKeyPrefix={`${blockKeyPrefix}-tag`}
                registerFieldRef={registerFieldRef}
                onFocusKeyChange={setActiveFieldKey}
                onAdd={() => addBlockStringListItem(commandIndex, blockIndex, 'tags')}
                onChange={(itemIndex, value) =>
                  updateBlockStringList(commandIndex, blockIndex, 'tags', itemIndex, value)
                }
                onRemove={(itemIndex) =>
                  removeBlockStringListItem(commandIndex, blockIndex, 'tags', itemIndex)
                }
                getVariableDropHandlers={(fieldKey, currentValue, onFieldChange) =>
                  bindVariableDropHandlers(
                    commandIndex,
                    blockIndex,
                    block,
                    fieldKey,
                    currentValue,
                    onFieldChange
                  )
                }
              />
              <AgentCommandStringListEditor
                label="附带图片"
                placeholder="https://example.com/help.png"
                values={block.images}
                fieldKeyPrefix={`${blockKeyPrefix}-image`}
                registerFieldRef={registerFieldRef}
                onFocusKeyChange={setActiveFieldKey}
                onAdd={() => addBlockStringListItem(commandIndex, blockIndex, 'images')}
                onChange={(itemIndex, value) =>
                  updateBlockStringList(commandIndex, blockIndex, 'images', itemIndex, value)
                }
                onRemove={(itemIndex) =>
                  removeBlockStringListItem(commandIndex, blockIndex, 'images', itemIndex)
                }
                getVariableDropHandlers={(fieldKey, currentValue, onFieldChange) =>
                  bindVariableDropHandlers(
                    commandIndex,
                    blockIndex,
                    block,
                    fieldKey,
                    currentValue,
                    onFieldChange
                  )
                }
              />
            </div>
          )}

          {block.kind === 'start_submission_session' && (
            <p className="field-hint">
              这个积木会为当前用户开启私聊投稿会话，本身不会自动发送提示文案。通常建议下一个积木接一条“回复私聊消息”。
            </p>
          )}

          {block.kind === 'finish_submission_session' && (
            <p className="field-hint">
              这个积木会把当前投稿会话标记为待确认，适合配合自定义回链文案，让用户确认后再提交。
            </p>
          )}

          {block.kind === 'resume_submission_session' && (
            <p className="field-hint">
              这个积木会把当前投稿会话恢复到继续编辑状态，适合用户要求补充内容后使用。
            </p>
          )}

          {block.kind === 'submit_submission_session' && (
            <p className="field-hint">
              这个积木会把当前私聊投稿会话直接提交到后端，进入正常审核与发送链路。
            </p>
          )}

          {block.kind === 'cancel_submission_session' && (
            <p className="field-hint">
              这个积木会清空当前用户已缓存的私聊投稿会话内容，本身不会自动发送提示文案。
            </p>
          )}

          {block.kind === 'insert_queued_post' && (
            <div className="agent-command-block-grid">
              <div
                className="field-stack fragment-drop-target"
                {...bindVariableDropHandlers(
                  commandIndex,
                  blockIndex,
                  block,
                  `${blockKeyPrefix}-moving_post_code`,
                  block.moving_post_code,
                  (nextValue) =>
                    updateBlock(commandIndex, blockIndex, (current) => {
                      if (current.kind !== 'insert_queued_post') return current
                      return { ...current, moving_post_code: nextValue }
                    })
                )}
              >
                <span className="field-label">要移动的投稿编号</span>
                <Input
                  ref={(node) => registerFieldRef(`${blockKeyPrefix}-moving_post_code`, node)}
                  placeholder="例如 previous_post_code"
                  value={block.moving_post_code}
                  onFocus={() => setActiveFieldKey(`${blockKeyPrefix}-moving_post_code`)}
                  onChange={(event) =>
                    updateBlock(commandIndex, blockIndex, (current) => {
                      if (current.kind !== 'insert_queued_post') return current
                      return { ...current, moving_post_code: event.target.value }
                    })
                  }
                />
              </div>
              <div className="field-stack">
                <span className="field-label">插入位置</span>
                <HeroSelect
                  ariaLabel={`选择队列插入位置 ${commandIndex + 1}-${blockIndex + 1}`}
                  selectedKey={block.position}
                  options={AGENT_QUEUE_INSERT_POSITION_OPTIONS}
                  onSelect={(value) =>
                    updateBlock(commandIndex, blockIndex, (current) => {
                      if (current.kind !== 'insert_queued_post') return current
                      return {
                        ...current,
                        position: value as AgentCommandQueueInsertPosition,
                      }
                    })
                  }
                />
              </div>
              <div
                className="field-stack agent-command-block-span-2 fragment-drop-target"
                {...bindVariableDropHandlers(
                  commandIndex,
                  blockIndex,
                  block,
                  `${blockKeyPrefix}-anchor_post_code`,
                  block.anchor_post_code,
                  (nextValue) =>
                    updateBlock(commandIndex, blockIndex, (current) => {
                      if (current.kind !== 'insert_queued_post') return current
                      return { ...current, anchor_post_code: nextValue }
                    })
                )}
              >
                <span className="field-label">目标投稿编号</span>
                <Input
                  ref={(node) => registerFieldRef(`${blockKeyPrefix}-anchor_post_code`, node)}
                  placeholder="例如 12345"
                  value={block.anchor_post_code}
                  onFocus={() => setActiveFieldKey(`${blockKeyPrefix}-anchor_post_code`)}
                  onChange={(event) =>
                    updateBlock(commandIndex, blockIndex, (current) => {
                      if (current.kind !== 'insert_queued_post') return current
                      return { ...current, anchor_post_code: event.target.value }
                    })
                  }
                />
              </div>
            </div>
          )}

          {block.kind === 'execute_review_action' &&
            renderReviewActionFields(commandIndex, blockIndex, block)}

          {block.kind === 'execute_global_action' &&
            renderGlobalActionFields(commandIndex, blockIndex, block)}

          {block.kind === 'send_webhook' && (
            <div className="agent-command-block-grid">
              <div
                className="field-stack fragment-drop-target"
                {...bindVariableDropHandlers(
                  commandIndex,
                  blockIndex,
                  block,
                  `${blockKeyPrefix}-url`,
                  block.url,
                  (nextValue) =>
                    updateRichMessageBlock(commandIndex, blockIndex, (current) => {
                      if (current.kind !== 'send_webhook') return current
                      return { ...current, url: nextValue }
                    })
                )}
              >
                <span className="field-label">Webhook 地址</span>
                <Input
                  ref={(node) => registerFieldRef(`${blockKeyPrefix}-url`, node)}
                  value={block.url}
                  onFocus={() => setActiveFieldKey(`${blockKeyPrefix}-url`)}
                  onChange={(event) =>
                    updateRichMessageBlock(commandIndex, blockIndex, (current) => {
                      if (current.kind !== 'send_webhook') return current
                      return { ...current, url: event.target.value }
                    })
                  }
                  />
                </div>
              <div
                className="field-stack fragment-drop-target"
                {...bindVariableDropHandlers(
                  commandIndex,
                  blockIndex,
                  block,
                  `${blockKeyPrefix}-source_webhook`,
                  block.source_webhook,
                  (nextValue) =>
                    updateRichMessageBlock(commandIndex, blockIndex, (current) => {
                      if (current.kind !== 'send_webhook') return current
                      return { ...current, source_webhook: nextValue }
                    })
                )}
              >
                <span className="field-label">source_webhook</span>
                <Input
                  ref={(node) => registerFieldRef(`${blockKeyPrefix}-source_webhook`, node)}
                  placeholder="例如 hook://agent"
                  value={block.source_webhook}
                  onFocus={() => setActiveFieldKey(`${blockKeyPrefix}-source_webhook`)}
                  onChange={(event) =>
                    updateRichMessageBlock(commandIndex, blockIndex, (current) => {
                      if (current.kind !== 'send_webhook') return current
                      return { ...current, source_webhook: event.target.value }
                    })
                  }
                  />
                </div>
              <div
                className="field-stack agent-command-block-span-2 fragment-drop-target"
                {...bindVariableDropHandlers(
                  commandIndex,
                  blockIndex,
                  block,
                  `${blockKeyPrefix}-text_template`,
                  block.text_template,
                  (nextValue) =>
                    updateRichMessageBlock(commandIndex, blockIndex, (current) => ({
                      ...current,
                      text_template: nextValue,
                    }))
                )}
              >
                <span className="field-label">Webhook 文本</span>
                <TextArea
                  ref={(node) => registerFieldRef(`${blockKeyPrefix}-text_template`, node)}
                  value={block.text_template}
                  onFocus={() => setActiveFieldKey(`${blockKeyPrefix}-text_template`)}
                  onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                    updateRichMessageBlock(commandIndex, blockIndex, (current) => ({
                      ...current,
                      text_template: event.target.value,
                    }))
                  }
                />
              </div>
              <AgentCommandStringListEditor
                label="Webhook 标签"
                placeholder="例如 help 或 sender_id"
                values={block.tags}
                fieldKeyPrefix={`${blockKeyPrefix}-tag`}
                registerFieldRef={registerFieldRef}
                onFocusKeyChange={setActiveFieldKey}
                onAdd={() => addBlockStringListItem(commandIndex, blockIndex, 'tags')}
                onChange={(itemIndex, value) =>
                  updateBlockStringList(commandIndex, blockIndex, 'tags', itemIndex, value)
                }
                onRemove={(itemIndex) =>
                  removeBlockStringListItem(commandIndex, blockIndex, 'tags', itemIndex)
                }
                getVariableDropHandlers={(fieldKey, currentValue, onFieldChange) =>
                  bindVariableDropHandlers(
                    commandIndex,
                    blockIndex,
                    block,
                    fieldKey,
                    currentValue,
                    onFieldChange
                  )
                }
              />
              <AgentCommandStringListEditor
                label="Webhook 图片"
                placeholder="https://example.com/help.png"
                values={block.images}
                fieldKeyPrefix={`${blockKeyPrefix}-image`}
                registerFieldRef={registerFieldRef}
                onFocusKeyChange={setActiveFieldKey}
                onAdd={() => addBlockStringListItem(commandIndex, blockIndex, 'images')}
                onChange={(itemIndex, value) =>
                  updateBlockStringList(commandIndex, blockIndex, 'images', itemIndex, value)
                }
                onRemove={(itemIndex) =>
                  removeBlockStringListItem(commandIndex, blockIndex, 'images', itemIndex)
                }
                getVariableDropHandlers={(fieldKey, currentValue, onFieldChange) =>
                  bindVariableDropHandlers(
                    commandIndex,
                    blockIndex,
                    block,
                    fieldKey,
                    currentValue,
                    onFieldChange
                  )
                }
              />
            </div>
          )}
        </div>
        <div
          className={`agent-block-dropzone ${isAfterDropActive ? 'is-active' : ''}`}
          onDragOver={(event) => event.preventDefault()}
          onDragEnter={handleDropZoneEnter(commandIndex, blockIndex, 'after')}
          onDrop={performDrop(commandIndex, blockIndex, 'after')}
        />
      </div>
    )
  }

  function renderCommandCard(command: AppConfigAgentCommand, commandIndex: number) {
    const emptyDropActive =
      dragState?.commandIndex === commandIndex && dragState?.blockIndex === null

    return (
      <div key={`agent-command-${commandIndex}`} className="agent-command-card">
        <div className="agent-command-head">
          <div className="agent-command-grid">
            <div className="field-stack">
              <span className="field-label">触发词</span>
              <Input
                placeholder="例如 help"
                value={command.name}
                onChange={(event) =>
                  updateCommand(commandIndex, (item) => ({
                    ...item,
                    name: event.target.value,
                  }))
                }
              />
            </div>
            <div className="field-stack">
              <span className="field-label">说明</span>
              <Input
                placeholder="例如：给用户返回帮助菜单"
                value={command.description}
                onChange={(event) =>
                  updateCommand(commandIndex, (item) => ({
                    ...item,
                    description: event.target.value,
                  }))
                }
              />
            </div>
            <div className="field-stack settings-switch-stack">
              <span className="field-label">启用</span>
              <Switch
                isSelected={command.enabled}
                size="sm"
                onChange={(value) =>
                  updateCommand(commandIndex, (item) => ({ ...item, enabled: value }))
                }
              >
                {command.enabled ? '已启用' : '已关闭'}
              </Switch>
            </div>
          </div>
          <div className="agent-command-actions">
            <Button
              size="sm"
              variant="secondary"
              isDisabled={commandIndex === 0}
              onClick={() => moveCommand(commandIndex, -1)}
            >
              上移
            </Button>
            <Button
              size="sm"
              variant="secondary"
              isDisabled={commandIndex >= commands.length - 1}
              onClick={() => moveCommand(commandIndex, 1)}
            >
              下移
            </Button>
            <Button size="sm" variant="secondary" onClick={() => addBlock(commandIndex)}>
              新增积木
            </Button>
            <Button size="sm" variant="tertiary" onClick={() => removeCommand(commandIndex)}>
              删除指令
            </Button>
          </div>
        </div>

        {command.blocks.length > 0 ? (
          <div className="agent-command-block-list">
            {command.blocks.map((block, blockIndex) =>
              renderCommandBlock(command, commandIndex, block, blockIndex)
            )}
          </div>
        ) : (
          <div
            className={`settings-empty${emptyDropActive ? ' is-active' : ''}`}
            onDragOver={(event) => event.preventDefault()}
            onDragEnter={handleDropZoneEnter(commandIndex, null, 'before')}
            onDrop={performDrop(commandIndex, null, 'before')}
          >
            <MessageSquare size={20} />
            <span>当前指令还没有积木。你可以点击“新增积木”，或者把上方积木拖到这里。</span>
          </div>
        )}
      </div>
    )
  }

  return (
    <div className="settings-panel agent-workbench">
      <div className="settings-section-head">
        <div>
          <span className="field-label">用户 Agent 指令</span>
          <p className="field-hint">
            当用户发送 <code>#指令名</code> 或 <code>#指令名 参数</code> 时，会按下方积木顺序执行。变量碎片可以拖到任意字段，也可以点击后插入当前光标处。
          </p>
        </div>
        <Button size="sm" variant="secondary" onClick={addCommand}>
          新增指令
        </Button>
      </div>

      <div className="agent-block-library-group agent-fragment-panel">
        <div className="settings-section-head">
          <div>
            <span className="field-label">变量碎片</span>
            <p className="field-hint">把变量当成可拼接的碎片使用，拖到任意积木或输入框里即可。</p>
          </div>
          <span className="agent-drag-hint">可点击 / 可拖拽</span>
        </div>
        <AgentVariableFragmentPalette variables={variableFragments} onPick={pickVariableFragment} />
      </div>

      <div className="agent-block-library">
        {paletteGroups.map((group) => (
          <div key={group.title} className="agent-block-library-group">
            <div className="settings-section-head">
              <span className="field-label">{group.title}</span>
            </div>
            <div className="agent-block-library-grid">
              {group.options.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  draggable
                  className="agent-block-library-item"
                  onDragStart={(event) =>
                    setDragPayload(event, {
                      source: 'palette',
                      option: option.value as AgentCommandBlockOptionValue,
                    })
                  }
                  onDragEnd={() => setDragState(null)}
                >
                  <strong>{option.label}</strong>
                  <span>拖到下方工作区即可添加</span>
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>

      {commands.length > 0 ? (
        <div className="agent-command-list">
          {commands.map((command, commandIndex) => renderCommandCard(command, commandIndex))}
        </div>
      ) : (
        <div className="settings-empty">
          <MessageSquare size={20} />
          <span>当前分组还没有配置用户 Agent 指令。</span>
        </div>
      )}
    </div>
  )
  /*

  return (
    <div className="settings-panel">
      <div className="settings-section-head">
        <div>
          <span className="field-label">用户 Agent 指令</span>
          <p className="field-hint">
            当用户私聊发送 <code>#指令名</code> 或 <code>#指令名 参数</code> 时，按顺序执行下面的积木块。
            现在所有可执行的后端操作都可以在这里用积木搭出来。
          </p>
        </div>
        <Button size="sm" variant="secondary" onClick={addCommand}>
          新增指令
        </Button>
      </div>

      <div className="agent-block-library">
        {paletteGroups.map((group) => (
          <div key={group.title} className="agent-block-library-group">
            <div className="settings-section-head">
              <span className="field-label">{group.title}</span>
            </div>
            <div className="agent-block-library-grid">
              {group.options.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  draggable
                  className="agent-block-library-item"
                  onDragStart={(event) =>
                    setDragPayload(event, {
                      source: 'palette',
                      option: option.value as AgentCommandBlockOptionValue,
                    })
                  }
                >
                  <strong>{option.label}</strong>
                  <span>拖到下方工作区即可添加</span>
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>

      {commands.length > 0 && (
        <div className="agent-command-list">
          {commands.map((command, commandIndex) => (
            <div key={`agent-command-${commandIndex}`} className="agent-command-card">
              <div className="agent-command-head">
                <div className="agent-command-grid">
                  <div className="field-stack">
                    <span className="field-label">触发词</span>
                    <Input
                      placeholder="例如 help"
                      value={command.name}
                      onChange={(event) =>
                        updateCommand(commandIndex, (item) => ({
                          ...item,
                          name: event.target.value,
                        }))
                      }
                    />
                  </div>
                  <div className="field-stack">
                    <span className="field-label">说明</span>
                    <Input
                      placeholder="例如：给用户返回帮助菜单"
                      value={command.description}
                      onChange={(event) =>
                        updateCommand(commandIndex, (item) => ({
                          ...item,
                          description: event.target.value,
                        }))
                      }
                    />
                  </div>
                  <div className="field-stack settings-switch-stack">
                    <span className="field-label">启用</span>
                    <Switch
                      isSelected={command.enabled}
                      size="sm"
                      onChange={(value) =>
                        updateCommand(commandIndex, (item) => ({ ...item, enabled: value }))
                      }
                    >
                      {command.enabled ? '已启用' : '已关闭'}
                    </Switch>
                  </div>
                </div>
                <div className="agent-command-actions">
                  <Button
                    size="sm"
                    variant="secondary"
                    isDisabled={commandIndex === 0}
                    onClick={() => moveCommand(commandIndex, -1)}
                  >
                    上移
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    isDisabled={commandIndex >= commands.length - 1}
                    onClick={() => moveCommand(commandIndex, 1)}
                  >
                    下移
                  </Button>
                  <Button size="sm" variant="secondary" onClick={() => addBlock(commandIndex)}>
                    新增积木
                  </Button>
                  <Button size="sm" variant="tertiary" onClick={() => removeCommand(commandIndex)}>
                    删除指令
                  </Button>
                </div>
              </div>

              {command.blocks.length > 0 && (
                <div className="agent-command-block-list">
                  {command.blocks.map((block, blockIndex) => {
                    return (
                      <div key={`agent-command-${commandIndex}-block-${blockIndex}`}>
                        <div
                          className={`agent-block-dropzone ${
                            dragState?.commandIndex === commandIndex &&
                            dragState?.blockIndex === blockIndex &&
                            dragState?.position === 'before'
                              ? 'is-active'
                              : ''
                          }`}
                          onDragOver={(event) => event.preventDefault()}
                          onDragEnter={handleDropZoneEnter(commandIndex, blockIndex, 'before')}
                          onDrop={performDrop(commandIndex, blockIndex, 'before')}
                        />
                        <div
                          className="agent-command-block"
                          draggable
                          onDragStart={(event) =>
                            setDragPayload(event, {
                              source: 'workspace',
                              commandIndex,
                              blockIndex,
                            })
                          }
                          onDragEnd={() => setDragState(null)}
                        >
                        <div className="agent-command-block-head">
                          <div className="field-stack">
                            <span className="field-label">积木 {blockIndex + 1}</span>
                            <HeroSelect
                              ariaLabel={`选择指令 ${commandIndex + 1} 的积木 ${blockIndex + 1}`}
                              selectedKey={buildAgentCommandBlockSelectValue(block)}
                              options={AGENT_COMMAND_BLOCK_OPTIONS}
                              onSelect={(value) => changeBlockKind(commandIndex, blockIndex, value)}
                            />
                          </div>
                          <div className="agent-command-actions">
                            <span className="agent-drag-hint">拖拽排序</span>
                            <Button
                              size="sm"
                              variant="secondary"
                              isDisabled={blockIndex === 0}
                              onClick={() => moveBlock(commandIndex, blockIndex, -1)}
                            >
                              上移
                            </Button>
                            <Button
                              size="sm"
                              variant="secondary"
                              isDisabled={blockIndex >= command.blocks.length - 1}
                              onClick={() => moveBlock(commandIndex, blockIndex, 1)}
                            >
                              下移
                            </Button>
                            <Button
                              size="sm"
                              variant="tertiary"
                              onClick={() => removeBlock(commandIndex, blockIndex)}
                            >
                              删除积木
                            </Button>
                          </div>
                        </div>

                        {block.kind === 'reply_private_message' && (
                          <div className="agent-command-block-grid">
                            <div className="field-stack agent-command-block-span-2">
                              <span className="field-label">回复文案</span>
                              <TextArea
                                ref={(node) =>
                                  registerFieldRef(
                                    `agent-${commandIndex}-${blockIndex}-text_template`,
                                    node
                                  )
                                }
                                value={block.text_template}
                                onFocus={() =>
                                  setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-text_template`)
                                }
                                onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                                  updateRichMessageBlock(commandIndex, blockIndex, (current) => ({
                                    ...current,
                                    text_template: event.target.value,
                                  }))
                                }
                              />
                            </div>
                            <AgentCommandStringListEditor
                              label="额外标签"
                              placeholder="例如 help 或 <sender_id>"
                              values={block.tags}
                              fieldKeyPrefix={`agent-${commandIndex}-${blockIndex}-tag`}
                              registerFieldRef={registerFieldRef}
                              onFocusKeyChange={setActiveFieldKey}
                              onAdd={() => addBlockStringListItem(commandIndex, blockIndex, 'tags')}
                              onChange={(itemIndex, value) =>
                                updateBlockStringList(
                                  commandIndex,
                                  blockIndex,
                                  'tags',
                                  itemIndex,
                                  value
                                )
                              }
                              onRemove={(itemIndex) =>
                                removeBlockStringListItem(
                                  commandIndex,
                                  blockIndex,
                                  'tags',
                                  itemIndex
                                )
                              }
                            />
                            <AgentCommandStringListEditor
                              label="附带图片"
                              placeholder="https://example.com/help.png"
                              values={block.images}
                              fieldKeyPrefix={`agent-${commandIndex}-${blockIndex}-image`}
                              registerFieldRef={registerFieldRef}
                              onFocusKeyChange={setActiveFieldKey}
                              onAdd={() => addBlockStringListItem(commandIndex, blockIndex, 'images')}
                              onChange={(itemIndex, value) =>
                                updateBlockStringList(
                                  commandIndex,
                                  blockIndex,
                                  'images',
                                  itemIndex,
                                  value
                                )
                              }
                              onRemove={(itemIndex) =>
                                removeBlockStringListItem(
                                  commandIndex,
                                  blockIndex,
                                  'images',
                                  itemIndex
                                )
                              }
                            />
                          </div>
                        )}

                        {block.kind === 'start_submission_session' && (
                          <p className="field-hint">
                            这个积木会为当前用户开启私聊投稿会话，本身不会自动发送提示文案。通常建议下一个积木接一条“回复私聊消息”。
                          </p>
                        )}

                        {block.kind === 'finish_submission_session' && (
                          <p className="field-hint">
                            这个积木会把当前投稿会话标记为待确认，适合配合自定义回链文案，让用户确认后再提交。
                          </p>
                        )}

                        {block.kind === 'resume_submission_session' && (
                          <p className="field-hint">
                            这个积木会把当前投稿会话恢复到继续编辑状态，适合用户要求补充内容后使用。
                          </p>
                        )}

                        {block.kind === 'submit_submission_session' && (
                          <p className="field-hint">
                            这个积木会把当前私聊投稿会话直接提交到后端，进入正常审核与发送链路。
                          </p>
                        )}

                        {block.kind === 'cancel_submission_session' && (
                          <p className="field-hint">
                            这个积木会清空当前用户已缓存的私聊投稿会话内容，本身不会自动发送提示文案。
                          </p>
                        )}

                        {block.kind === 'insert_queued_post' && (
                          <div className="agent-command-block-grid">
                            <div className="field-stack">
                              <span className="field-label">要移动的投稿编号</span>
                              <Input
                                ref={(node) =>
                                  registerFieldRef(
                                    `agent-${commandIndex}-${blockIndex}-moving_post_code`,
                                    node
                                  )
                                }
                                placeholder="例如 <previous_post_code>"
                                value={block.moving_post_code}
                                onFocus={() =>
                                  setActiveFieldKey(
                                    `agent-${commandIndex}-${blockIndex}-moving_post_code`
                                  )
                                }
                                onChange={(event) =>
                                  updateBlock(commandIndex, blockIndex, (current) => {
                                    if (current.kind !== 'insert_queued_post') return current
                                    return { ...current, moving_post_code: event.target.value }
                                  })
                                }
                              />
                            </div>
                            <div className="field-stack">
                              <span className="field-label">插入位置</span>
                              <HeroSelect
                                ariaLabel={`选择队列插入位置 ${commandIndex + 1}-${blockIndex + 1}`}
                                selectedKey={block.position}
                                options={AGENT_QUEUE_INSERT_POSITION_OPTIONS}
                                onSelect={(value) =>
                                  updateBlock(commandIndex, blockIndex, (current) => {
                                    if (current.kind !== 'insert_queued_post') return current
                                    return {
                                      ...current,
                                      position: value as AgentCommandQueueInsertPosition,
                                    }
                                  })
                                }
                              />
                            </div>
                            <div className="field-stack agent-command-block-span-2">
                              <span className="field-label">目标投稿编号</span>
                              <Input
                                ref={(node) =>
                                  registerFieldRef(
                                    `agent-${commandIndex}-${blockIndex}-anchor_post_code`,
                                    node
                                  )
                                }
                                placeholder="例如 12345"
                                value={block.anchor_post_code}
                                onFocus={() =>
                                  setActiveFieldKey(
                                    `agent-${commandIndex}-${blockIndex}-anchor_post_code`
                                  )
                                }
                                onChange={(event) =>
                                  updateBlock(commandIndex, blockIndex, (current) => {
                                    if (current.kind !== 'insert_queued_post') return current
                                    return { ...current, anchor_post_code: event.target.value }
                                  })
                                }
                              />
                            </div>
                          </div>
                        )}

                        {block.kind === 'execute_review_action' &&
                          renderReviewActionFields(commandIndex, blockIndex, block)}

                        {block.kind === 'execute_global_action' &&
                          renderGlobalActionFields(commandIndex, blockIndex, block)}

                        {block.kind === 'send_webhook' && (
                          <div className="agent-command-block-grid">
                            <div className="field-stack">
                              <span className="field-label">Webhook 地址</span>
                              <Input
                                ref={(node) =>
                                  registerFieldRef(`agent-${commandIndex}-${blockIndex}-url`, node)
                                }
                                value={block.url}
                                onFocus={() =>
                                  setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-url`)
                                }
                                onChange={(event) =>
                                  updateRichMessageBlock(commandIndex, blockIndex, (current) => {
                                    if (current.kind !== 'send_webhook') return current
                                    return { ...current, url: event.target.value }
                                  })
                                }
                              />
                            </div>
                            <div className="field-stack">
                              <span className="field-label">source_webhook</span>
                              <Input
                                ref={(node) =>
                                  registerFieldRef(
                                    `agent-${commandIndex}-${blockIndex}-source_webhook`,
                                    node
                                  )
                                }
                                placeholder="例如 hook://agent"
                                value={block.source_webhook}
                                onFocus={() =>
                                  setActiveFieldKey(
                                    `agent-${commandIndex}-${blockIndex}-source_webhook`
                                  )
                                }
                                onChange={(event) =>
                                  updateRichMessageBlock(commandIndex, blockIndex, (current) => {
                                    if (current.kind !== 'send_webhook') return current
                                    return { ...current, source_webhook: event.target.value }
                                  })
                                }
                              />
                            </div>
                            <div className="field-stack agent-command-block-span-2">
                              <span className="field-label">Webhook 文本</span>
                              <TextArea
                                ref={(node) =>
                                  registerFieldRef(
                                    `agent-${commandIndex}-${blockIndex}-text_template`,
                                    node
                                  )
                                }
                                value={block.text_template}
                                onFocus={() =>
                                  setActiveFieldKey(`agent-${commandIndex}-${blockIndex}-text_template`)
                                }
                                onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                                  updateRichMessageBlock(commandIndex, blockIndex, (current) => ({
                                    ...current,
                                    text_template: event.target.value,
                                  }))
                                }
                              />
                            </div>
                            <AgentCommandStringListEditor
                              label="Webhook 标签"
                              placeholder="例如 help 或 <sender_id>"
                              values={block.tags}
                              fieldKeyPrefix={`agent-${commandIndex}-${blockIndex}-tag`}
                              registerFieldRef={registerFieldRef}
                              onFocusKeyChange={setActiveFieldKey}
                              onAdd={() => addBlockStringListItem(commandIndex, blockIndex, 'tags')}
                              onChange={(itemIndex, value) =>
                                updateBlockStringList(
                                  commandIndex,
                                  blockIndex,
                                  'tags',
                                  itemIndex,
                                  value
                                )
                              }
                              onRemove={(itemIndex) =>
                                removeBlockStringListItem(
                                  commandIndex,
                                  blockIndex,
                                  'tags',
                                  itemIndex
                                )
                              }
                            />
                            <AgentCommandStringListEditor
                              label="Webhook 图片"
                              placeholder="https://example.com/help.png"
                              values={block.images}
                              fieldKeyPrefix={`agent-${commandIndex}-${blockIndex}-image`}
                              registerFieldRef={registerFieldRef}
                              onFocusKeyChange={setActiveFieldKey}
                              onAdd={() => addBlockStringListItem(commandIndex, blockIndex, 'images')}
                              onChange={(itemIndex, value) =>
                                updateBlockStringList(
                                  commandIndex,
                                  blockIndex,
                                  'images',
                                  itemIndex,
                                  value
                                )
                              }
                              onRemove={(itemIndex) =>
                                removeBlockStringListItem(
                                  commandIndex,
                                  blockIndex,
                                  'images',
                                  itemIndex
                                )
                              }
                            />
                          </div>
                        )}
                      </div>
                    )
                  })}
                </div>
              )}

              {command.blocks.length === 0 && (
                <div className="settings-empty">
                  <MessageSquare size={20} />
                  <span>当前指令还没有积木。至少添加一个积木后再保存。</span>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {commands.length === 0 && (
        <div className="settings-empty">
          <MessageSquare size={20} />
          <span>当前分组还没有配置用户 Agent 指令。</span>
        </div>
      )}
    </div>
  )
}

 */

}

function AgentCommandStringListEditor({
  label,
  placeholder,
  values,
  fieldKeyPrefix,
  registerFieldRef,
  onFocusKeyChange,
  onAdd,
  onChange,
  onRemove,
  getVariableDropHandlers,
}: {
  label: string
  placeholder: string
  values: string[]
  fieldKeyPrefix: string
  registerFieldRef: (key: string, node: AgentCommandFieldNode | null) => void
  onFocusKeyChange: (key: string | null) => void
  onAdd: () => void
  onChange: (index: number, value: string) => void
  onRemove: (index: number) => void
  getVariableDropHandlers?: (
    fieldKey: string,
    currentValue: string,
    onFieldChange: (nextValue: string) => void
  ) => {
    onDragOver: (event: DragEvent<HTMLElement>) => void
    onDrop: (event: DragEvent<HTMLElement>) => void
  }
}) {
  return (
    <div className="field-stack">
      <div className="settings-section-head">
        <span className="field-label">{label}</span>
        <Button size="sm" variant="secondary" onClick={onAdd}>
          新增
        </Button>
      </div>
      {values.length ? (
        <div className="settings-image-list">
          {values.map((value, index) => {
            const fieldKey = `${fieldKeyPrefix}-${index}`
            const variableDropHandlers = getVariableDropHandlers
              ? getVariableDropHandlers(fieldKey, value, (nextValue) => onChange(index, nextValue))
              : null
            return (
              <div
                key={fieldKey}
                className={`settings-image-item${variableDropHandlers ? ' fragment-drop-target' : ''}`}
                {...variableDropHandlers}
              >
                <div className="settings-image-row">
                  <Input
                    ref={(node) => registerFieldRef(fieldKey, node)}
                    className="settings-image-input"
                    placeholder={placeholder}
                    value={value}
                    onFocus={() => onFocusKeyChange(fieldKey)}
                    onChange={(event) => onChange(index, event.target.value)}
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
            )
          })}
        </div>
      ) : (
        <div className="settings-empty">
          <MessageSquare size={20} />
          <span>当前还没有配置{label}。</span>
        </div>
      )}
    </div>
  )
}

function buildDefaultAgentCommand(name: string): AppConfigAgentCommand {
  return {
    name,
    enabled: true,
    description: '',
    blocks: [buildDefaultAgentCommandBlock('reply_private_message')],
  }
}

function buildDefaultReviewAction(action: AgentReviewActionKey): AgentCommandReviewAction {
  switch (action) {
    case 'approve':
      return { action }
    case 'reject':
      return { action }
    case 'delete':
      return { action }
    case 'defer':
      return { action, delay_ms: '180000' }
    case 'skip':
      return { action }
    case 'immediate':
      return { action }
    case 'refresh':
      return { action }
    case 'rerender':
      return { action }
    case 'select_all_messages':
      return { action }
    case 'toggle_anonymous':
      return { action }
    case 'expand_audit':
      return { action }
    case 'show':
      return { action }
    case 'comment':
      return { action, text_template: '' }
    case 'reply':
      return { action, text_template: '' }
    case 'blacklist':
      return { action, reason_template: '' }
    case 'quick_reply':
      return { action, key_template: '' }
    case 'merge':
      return { action, target_review_code: '' }
  }
}

function buildDefaultGlobalAction(action: AgentGlobalActionKey): AgentCommandGlobalAction {
  switch (action) {
    case 'help':
      return { action }
    case 'recall':
      return { action, review_code: '' }
    case 'withdraw':
      return { action, review_code: '' }
    case 'info':
      return { action, review_code: '' }
    case 'manual_relogin':
      return { action }
    case 'auto_relogin':
      return { action }
    case 'pending_list':
      return { action }
    case 'pending_clear':
      return { action }
    case 'send_queue_clear':
      return { action }
    case 'send_queue_flush':
      return { action }
    case 'send_in_flight_clear':
      return { action }
    case 'blacklist_list':
      return { action }
    case 'blacklist_add':
      return { action, sender_id: '', reason_template: '' }
    case 'blacklist_remove':
      return { action, sender_id: '' }
    case 'set_external_number':
      return { action, value_template: '' }
    case 'quick_reply_list':
      return { action }
    case 'quick_reply_add':
      return { action, key_template: '', text_template: '' }
    case 'quick_reply_delete':
      return { action, key_template: '' }
    case 'shortcut_list':
      return { action }
    case 'shortcut_add':
      return { action, scope: 'review', key_template: '', definition_template: '' }
    case 'shortcut_delete':
      return { action, scope: 'review', key_template: '' }
    case 'self_check':
      return { action }
    case 'system_repair':
      return { action }
  }
}

function buildDefaultAgentCommandBlock(
  kind: AgentCommandBlock['kind'] | AgentCommandBlockOptionValue
): AgentCommandBlock {
  if (kind.startsWith('review:')) {
    return {
      kind: 'execute_review_action',
      review_code: '',
      action: buildDefaultReviewAction(kind.slice('review:'.length) as AgentReviewActionKey),
    }
  }
  if (kind.startsWith('global:')) {
    return {
      kind: 'execute_global_action',
      action: buildDefaultGlobalAction(kind.slice('global:'.length) as AgentGlobalActionKey),
    }
  }
  switch (kind) {
    case 'reply_private_message':
      return {
        kind,
        text_template: '',
        tags: [],
        images: [],
      }
    case 'start_submission_session':
      return { kind }
    case 'finish_submission_session':
      return { kind }
    case 'resume_submission_session':
      return { kind }
    case 'submit_submission_session':
      return { kind }
    case 'cancel_submission_session':
      return { kind }
    case 'insert_queued_post':
      return {
        kind,
        moving_post_code: '',
        anchor_post_code: '',
        position: 'before',
      }
    case 'send_webhook':
      return {
        kind,
        url: '',
        source_webhook: '',
        text_template: '',
        tags: [],
        images: [],
      }
    case 'execute_review_action':
      return {
        kind,
        review_code: '',
        action: buildDefaultReviewAction('approve'),
      }
    case 'execute_global_action':
      return {
        kind,
        action: buildDefaultGlobalAction('help'),
      }
    default:
      throw new Error(`Unsupported agent command block kind: ${kind}`)
  }
}

function convertAgentCommandReviewAction(
  action: AgentCommandReviewAction,
  nextAction: AgentReviewActionKey
): AgentCommandReviewAction {
  if (action.action === nextAction) return action
  return buildDefaultReviewAction(nextAction)
}

function convertAgentCommandGlobalAction(
  action: AgentCommandGlobalAction,
  nextAction: AgentGlobalActionKey
): AgentCommandGlobalAction {
  if (action.action === nextAction) return action
  return buildDefaultGlobalAction(nextAction)
}

function convertAgentCommandBlockKind(
  block: AgentCommandBlock,
  kind: AgentCommandBlockOptionValue
): AgentCommandBlock {
  if (kind.startsWith('review:')) {
    const actionKey = kind.slice('review:'.length) as AgentReviewActionKey
    if (block.kind === 'execute_review_action') {
      return {
        ...block,
        action: convertAgentCommandReviewAction(block.action, actionKey),
      }
    }
    return buildDefaultAgentCommandBlock(kind)
  }
  if (kind.startsWith('global:')) {
    const actionKey = kind.slice('global:'.length) as AgentGlobalActionKey
    if (block.kind === 'execute_global_action') {
      return {
        ...block,
        action: convertAgentCommandGlobalAction(block.action, actionKey),
      }
    }
    return buildDefaultAgentCommandBlock(kind)
  }
  if (block.kind === kind) return block
  if (kind === 'reply_private_message' && block.kind === 'send_webhook') {
    return {
      kind,
      text_template: block.text_template,
      tags: [...block.tags],
      images: [...block.images],
    }
  }
  if (kind === 'send_webhook' && block.kind === 'reply_private_message') {
    return {
      kind,
      url: '',
      source_webhook: '',
      text_template: block.text_template,
      tags: [...block.tags],
      images: [...block.images],
    }
  }
  return buildDefaultAgentCommandBlock(kind)
}

function buildNextAgentCommandName(commands: AppConfigAgentCommand[]) {
  let index = commands.length + 1
  while (commands.some((command) => command.name === `command_${index}`)) {
    index += 1
  }
  return `command_${index}`
}

function moveArrayItem<T>(items: T[], index: number, direction: -1 | 1) {
  const targetIndex = index + direction
  if (targetIndex < 0 || targetIndex >= items.length) return items
  const next = [...items]
  const [item] = next.splice(index, 1)
  next.splice(targetIndex, 0, item)
  return next
}

function buildDefaultConfigGroup(
  groupId: string,
  common: AppConfigCommonSettings
): AppConfigGroupSettings {
  return {
    group_id: groupId,
    audit_group_id: '',
    accounts: [''],
    napcat_base_url: common.napcat_base_url,
    napcat_access_token: common.napcat_access_token,
    process_waittime_sec: common.process_waittime_sec,
    min_interval_ms: common.min_interval_ms,
    max_post_stack: 1,
    max_image_number_one_post: common.max_image_number_one_post,
    send_timeout_ms: common.send_timeout_ms,
    send_max_attempts: common.send_max_attempts,
    send_schedule: [],
    individual_image_in_posts: true,
    watermark_text: '',
    friend_add_message: common.friend_add_message,
    friend_request_window_sec: common.friend_request_window_sec,
    quick_replies: [],
    review_shortcuts: [],
    global_shortcuts: [],
    agent_commands: [],
    webview_admins: [],
  }
}

function buildNextGroupId(groups: AppConfigGroupSettings[]) {
  let index = groups.length + 1
  while (groups.some((group) => group.group_id === `group_${index}`)) {
    index += 1
  }
  return `group_${index}`
}

function parseIntegerInput(
  raw: string,
  options: { allowNegative?: boolean; fallback?: number } = {}
) {
  const fallback = options.fallback ?? 0
  const parsed = Number(raw)
  if (!Number.isFinite(parsed)) return fallback
  const truncated = Math.trunc(parsed)
  return options.allowNegative ? truncated : Math.max(0, truncated)
}

function UserNotificationSettingsView({
  notify,
}: {
  notify: (kind: ToastKind, text: string) => void
}) {
  /*
  const [settings, setSettings] = useState<UserNotificationSettingsResponse | null>(null)
  const [selectedGroup, setSelectedGroup] = useState('')
  const [selectedStage, setSelectedStage] = useState<NotificationStageKey>('send_succeeded')
  const [selectedMenu, setSelectedMenu] = useState<NotificationMenuKey>('stages')
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const uploadInputRef = useRef<HTMLInputElement | null>(null)
  const templateTextareaRef = useRef<HTMLTextAreaElement | null>(null)

  useEffect(() => {
    void loadSettings()
  }, [])

  function applySettings(result: UserNotificationSettingsResponse) {
    setSettings(result)
    setSelectedGroup(result.group_id)
  }

  async function loadSettings(groupId?: string) {
    setLoading(true)
    try {
      const params = new URLSearchParams()
      const targetGroup = (groupId ?? selectedGroup).trim()
      if (targetGroup) params.set('group_id', targetGroup)
      const suffix = params.toString() ? `?${params.toString()}` : ''
      const result = await api<UserNotificationSettingsResponse>(
        `/api/settings/user-notifications${suffix}`
      )
      applySettings(result)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  async function saveSettings() {
    if (!settings || !selectedGroup) {
      notify('error', '请先选择要配置的群组')
      return
    }
    setSaving(true)
    try {
      const result = await api<UserNotificationSettingsResponse>(
        '/api/settings/user-notifications',
        {
          method: 'POST',
          body: JSON.stringify({
            group_id: selectedGroup,
            queue_entered: settings.queue_entered,
            review_queued: settings.review_queued,
            send_succeeded: settings.send_succeeded,
            rejected: settings.rejected,
            webhook_tag_map: settings.webhook_tag_map,
            tag_value_maps: settings.tag_value_maps,
          }),
        }
      )
      applySettings(result)
      notify('success', '回链设置已保存，并同步到运行时')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setSaving(false)
    }
  }

  function updateStageTemplate(
    updater: (template: UserNotificationTemplate) => UserNotificationTemplate
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [selectedStage]: updater(prev[selectedStage]),
      }
    })
  }

  function insertVariable(key: string) {
    const token = `<${key}>`
    const textarea = templateTextareaRef.current
    const currentText = activeTemplate.text_template
    const selectionStart = textarea?.selectionStart ?? currentText.length
    const selectionEnd = textarea?.selectionEnd ?? currentText.length
    const nextText = `${currentText.slice(0, selectionStart)}${token}${currentText.slice(selectionEnd)}`
    updateStageTemplate((template) => ({
      ...template,
      text_template: nextText,
    }))
    window.requestAnimationFrame(() => {
      const nextCaret = selectionStart + token.length
      templateTextareaRef.current?.focus()
      templateTextareaRef.current?.setSelectionRange(nextCaret, nextCaret)
    })
  }

  function updateStageList(field: 'tags' | 'images', index: number, value: string) {
    updateStageTemplate((template) => ({
      ...template,
      [field]: template[field].map((item, itemIndex) => (itemIndex === index ? value : item)),
    }))
  }

  function addStageListItem(field: 'tags' | 'images') {
    updateStageTemplate((template) => ({
      ...template,
      [field]: [...template[field], ''],
    }))
  }

  function removeStageListItem(field: 'tags' | 'images', index: number) {
    updateStageTemplate((template) => ({
      ...template,
      [field]: template[field].filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function updateMapping(
    field: 'webhook_tag_map',
    index: number,
    key: keyof MappingEntry,
    value: string
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: prev[field].map((item, itemIndex) =>
          itemIndex === index ? { ...item, [key]: value } : item
        ),
      }
    })
  }

  function addMapping(field: 'webhook_tag_map') {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: [...prev[field], { key: '', value: '' }],
      }
    })
  }

  function removeMapping(field: 'webhook_tag_map', index: number) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: prev[field].filter((_, itemIndex) => itemIndex !== index),
      }
    })
  }

  async function uploadImages(event: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? [])
    if (!files.length) return
    try {
      const uploaded = await Promise.all(files.map(readFileAsDataUrl))
      updateStageTemplate((template) => ({
        ...template,
        images: [...template.images, ...uploaded],
      }))
      notify('success', `已添加 ${uploaded.length} 张图片`)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      event.target.value = ''
    }
  }

  if (loading && !settings) {
    return <EmptyPanel icon={<Spinner />} text="正在加载用户回链设置" />
  }

  if (!settings) {
    return (
      <div className="workspace">
        <header className="page-head">
          <div>
            <h1>用户回链</h1>
            <p>配置进入队列、发稿成功、拒稿三种阶段的用户通知。</p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            重试
          </Button>
        </header>
        <Card className="panel-card">
          <Card.Content>
            <EmptyPanel icon={<Send size={28} />} text="当前没有可用的回链设置数据" />
          </Card.Content>
        </Card>
      </div>
    )
  }

  const activeStage = NOTIFICATION_STAGE_OPTIONS.find((item) => item.value === selectedStage)!
  const activeTemplate = settings[selectedStage]
  const groupOptions = settings.available_groups.map((group) => ({ value: group, label: group }))

  return (
    <div className="workspace settings-workspace">
      <header className="page-head">
        <div>
          <h1>用户回链</h1>
          <p>支持 webhook 映射标签、标签值重写，以及三阶段自定义文案、标签和图片。</p>
        </div>
        <div className="head-actions">
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading || saving}
            onClick={() => void loadSettings(selectedGroup)}
          >
            <RefreshCcw size={16} />
            刷新
          </Button>
          <Button size="sm" isDisabled={loading || saving} onClick={() => void saveSettings()}>
            {saving ? <Spinner size="sm" /> : <Check size={16} />}
            保存
          </Button>
        </div>
      </header>

      <section className="settings-grid">
        <Card className="panel-card settings-main-card">
          <Card.Header>
            <Card.Title>阶段模板</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-toolbar">
                <div className="field-stack">
                  <span className="field-label">群组</span>
                  <HeroSelect
                    className="control settings-group-select"
                    ariaLabel="选择群组"
                    selectedKey={selectedGroup}
                    options={groupOptions}
                    onSelect={(value) => {
                      setSelectedGroup(value)
                      void loadSettings(value)
                    }}
                  />
                </div>
              </div>

              <div className="settings-stage-grid">
                {NOTIFICATION_STAGE_OPTIONS.map((stage) => (
                  <button
                    key={stage.value}
                    type="button"
                    className={`settings-stage-button ${
                      selectedStage === stage.value ? 'is-active' : ''
                    }`}
                    onClick={() => setSelectedStage(stage.value)}
                  >
                    <span className="settings-stage-icon">{stage.icon}</span>
                    <strong>{stage.label}</strong>
                    <span>{stage.description}</span>
                  </button>
                ))}
              </div>

              <div className="settings-inline-grid">
                <div className="field-stack settings-switch-stack">
                  <span className="field-label">当前阶段</span>
                  <span className="field-hint">{activeStage.label}</span>
                </div>
                <div className="field-stack settings-switch-stack">
                  <span className="field-label">启用通知</span>
                  <Switch
                    isSelected={activeTemplate.enabled}
                    onChange={(value) =>
                      updateStageTemplate((template) => ({ ...template, enabled: value }))
                    }
                    size="sm"
                  >
                    {activeTemplate.enabled ? '已启用' : '已关闭'}
                  </Switch>
                </div>
                <div className="field-stack settings-switch-stack">
                  <span className="field-label">自动带上稿件标签</span>
                  <Switch
                    isSelected={activeTemplate.include_post_tags}
                    onChange={(value) =>
                      updateStageTemplate((template) => ({
                        ...template,
                        include_post_tags: value,
                      }))
                    }
                    size="sm"
                  >
                    {activeTemplate.include_post_tags ? '已启用' : '已关闭'}
                  </Switch>
                </div>
              </div>

              <div className="field-stack">
                <span className="field-label">通知文案</span>
                <TextArea
                  ref={templateTextareaRef}
                  className="settings-template-textarea"
                  placeholder="#<code> 已发稿"
                  value={activeTemplate.text_template}
                  onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                    updateStageTemplate((template) => ({
                      ...template,
                      text_template: event.target.value,
                    }))
                  }
                />
                <span className="field-hint">
                  支持使用 <code>{'<变量>'}</code> 占位。标签会显示在正文上方，图片按顺序附加。
                </span>
              </div>

              <div className="field-stack">
                <div className="settings-section-head">
                  <span className="field-label">可插入变量</span>
                  <span className="field-hint">点击后会在当前光标处插入变量，标签和图片地址同样支持变量。</span>
                </div>
                <div className="settings-variable-grid">
                  {settings.variables.map((variable) => (
                    <button
                      key={variable.key}
                      type="button"
                      className="settings-variable-button"
                      onClick={() => insertVariable(variable.key)}
                    >
                      <strong>{variable.label}</strong>
                      <code>{variable.example}</code>
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card settings-side-card">
          <Card.Header>
            <Card.Title>标签与图片</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-subsection">
                <div className="settings-section-head">
                  <span className="field-label">额外标签</span>
                  <Button size="sm" variant="secondary" onClick={() => addStageListItem('tags')}>
                    新增标签
                  </Button>
                </div>
                <span className="field-hint">
                  每个标签都支持变量，占位后会经过标签值映射表再发送。
                </span>
                {activeTemplate.tags.length ? (
                  <div className="settings-image-list">
                    {activeTemplate.tags.map((tag, index) => (
                      <div key={`tag-${selectedStage}-${index}`} className="settings-image-item">
                        <div className="settings-image-row">
                          <Input
                            className="settings-image-input"
                            placeholder="例如 <source_webhook_tag> 或 3344846508"
                            value={tag}
                            onChange={(event) => updateStageList('tags', index, event.target.value)}
                          />
                          <Button
                            size="sm"
                            variant="tertiary"
                            isIconOnly
                            aria-label={`删除标签 ${index + 1}`}
                            onClick={() => removeStageListItem('tags', index)}
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
                    <span>当前阶段未配置额外标签。</span>
                  </div>
                )}
              </div>

              <div className="settings-subsection">
                <div className="settings-image-actions">
                  <Button size="sm" variant="secondary" onClick={() => addStageListItem('images')}>
                    新增图片
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => uploadInputRef.current?.click()}
                  >
                    上传图片
                  </Button>
                  <input
                    ref={uploadInputRef}
                    className="sr-only"
                    type="file"
                    accept="image/*"
                    multiple
                    onChange={uploadImages}
                  />
                </div>
                <span className="field-hint">
                  可填写远程图片 URL，也可直接上传，上传后会保存为 data URL。
                </span>
                {activeTemplate.images.length ? (
                  <div className="settings-image-list">
                    {activeTemplate.images.map((image, index) => {
                      const previewable = isPreviewableImageSource(image)
                      return (
                        <div
                          key={`reply-image-${selectedStage}-${index}`}
                          className="settings-image-item"
                        >
                          <div className="settings-image-row">
                            <Input
                              className="settings-image-input"
                              placeholder="https://example.com/success.png 或 data:image/..."
                              value={image}
                              onChange={(event) =>
                                updateStageList('images', index, event.target.value)
                              }
                            />
                            <Button
                              size="sm"
                              variant="tertiary"
                              isIconOnly
                              aria-label={`删除图片 ${index + 1}`}
                              onClick={() => removeStageListItem('images', index)}
                            >
                              <Trash2 size={16} />
                            </Button>
                          </div>
                          <div className="settings-image-meta">
                            <span className="field-hint">{buildReplyImageLabel(image, index)}</span>
                          </div>
                          {previewable && (
                            <div className="settings-image-preview-frame">
                              <img
                                className="settings-image-preview"
                                src={image.trim()}
                                alt={`回链图片预览 ${index + 1}`}
                              />
                            </div>
                          )}
                        </div>
                      )
                    })}
                  </div>
                ) : (
                  <div className="settings-empty">
                    <MessageSquare size={20} />
                    <span>当前阶段未附带图片，保存后只发送标签和文案。</span>
                  </div>
                )}
              </div>
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card settings-side-card">
          <Card.Header>
            <Card.Title>Webhook 到标签</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-panel">
              <div className="settings-section-head">
                <span className="field-hint">将 webhook 标识映射为默认标签。</span>
                <Button size="sm" variant="secondary" onClick={() => addMapping('webhook_tag_map')}>
                  新增映射
                </Button>
              </div>
              {settings.webhook_tag_map.length ? (
                <div className="settings-variable-list">
                  {settings.webhook_tag_map.map((entry, index) => (
                    <div key={`webhook-map-${index}`} className="settings-variable-card">
                      <div className="settings-image-row">
                        <Input
                          className="settings-image-input"
                          placeholder="Webhook"
                          value={entry.key}
                          onChange={(event) =>
                            updateMapping('webhook_tag_map', index, 'key', event.target.value)
                          }
                        />
                        <Input
                          className="settings-image-input"
                          placeholder="标签"
                          value={entry.value}
                          onChange={(event) =>
                            updateMapping('webhook_tag_map', index, 'value', event.target.value)
                          }
                        />
                        <Button
                          size="sm"
                          variant="tertiary"
                          isIconOnly
                          aria-label={`删除 webhook 映射 ${index + 1}`}
                          onClick={() => removeMapping('webhook_tag_map', index)}
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
                  <span>当前没有 webhook 映射。</span>
                </div>
              )}
            </div>
          </Card.Content>
        </Card>

        <Card className="panel-card settings-side-card">
          <Card.Header>
            <Card.Title>变量说明</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="settings-variable-list">
              {settings.variables.map((variable) => (
                <div key={variable.key} className="settings-variable-card">
                  <div className="settings-variable-meta">
                    <strong>{variable.label}</strong>
                    <code>{variable.example}</code>
                  </div>
                  <p>{variable.description}</p>
                </div>
              ))}
            </div>
          </Card.Content>
        </Card>
      </section>
    </div>
  )
  */
  return <NotificationSettingsWorkbench notify={notify} />
}

function NotificationSettingsWorkbench({
  notify,
}: {
  notify: (kind: ToastKind, text: string) => void
}) {
  const [settings, setSettings] = useState<UserNotificationSettingsResponse | null>(null)
  const [selectedGroup, setSelectedGroup] = useState('')
  const [selectedStage, setSelectedStage] = useState<NotificationStageKey>('send_succeeded')
  const [selectedMenu, setSelectedMenu] = useState<NotificationMenuKey>('stages')
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const uploadInputRef = useRef<HTMLInputElement | null>(null)
  const templateTextareaRef = useRef<HTMLTextAreaElement | null>(null)

  useEffect(() => {
    void loadSettings()
  }, [])

  async function loadSettings(groupId?: string) {
    setLoading(true)
    try {
      const params = new URLSearchParams()
      const targetGroup = (groupId ?? selectedGroup).trim()
      if (targetGroup) params.set('group_id', targetGroup)
      const suffix = params.toString() ? `?${params.toString()}` : ''
      const result = await api<UserNotificationSettingsResponse>(
        `/api/settings/user-notifications${suffix}`
      )
      setSettings(result)
      setSelectedGroup(result.group_id)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  async function saveSettings() {
    if (!settings || !selectedGroup) {
      notify('error', '请先选择要配置的群组')
      return
    }
    setSaving(true)
    try {
      const result = await api<UserNotificationSettingsResponse>(
        '/api/settings/user-notifications',
        {
          method: 'POST',
          body: JSON.stringify({
            group_id: selectedGroup,
            queue_entered: settings.queue_entered,
            review_queued: settings.review_queued,
            send_succeeded: settings.send_succeeded,
            rejected: settings.rejected,
            webhook_tag_map: settings.webhook_tag_map,
          }),
        }
      )
      setSettings(result)
      setSelectedGroup(result.group_id)
      notify('success', '用户回链设置已保存')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setSaving(false)
    }
  }

  function updateStageTemplate(
    updater: (template: UserNotificationTemplate) => UserNotificationTemplate
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [selectedStage]: updater(prev[selectedStage]),
      }
    })
  }

  function updateStageList(field: 'tags' | 'images', index: number, value: string) {
    updateStageTemplate((template) => ({
      ...template,
      [field]: template[field].map((item, itemIndex) => (itemIndex === index ? value : item)),
    }))
  }

  function addStageListItem(field: 'tags' | 'images') {
    updateStageTemplate((template) => ({
      ...template,
      [field]: [...template[field], ''],
    }))
  }

  function removeStageListItem(field: 'tags' | 'images', index: number) {
    updateStageTemplate((template) => ({
      ...template,
      [field]: template[field].filter((_, itemIndex) => itemIndex !== index),
    }))
  }

  function updateMapping(
    field: 'webhook_tag_map',
    index: number,
    key: keyof MappingEntry,
    value: string
  ) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: prev[field].map((item, itemIndex) =>
          itemIndex === index ? { ...item, [key]: value } : item
        ),
      }
    })
  }

  function addMapping(field: 'webhook_tag_map') {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: [...prev[field], { key: '', value: '' }],
      }
    })
  }

  function removeMapping(field: 'webhook_tag_map', index: number) {
    setSettings((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        [field]: prev[field].filter((_, itemIndex) => itemIndex !== index),
      }
    })
  }

  async function uploadImages(event: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? [])
    if (!files.length) return
    try {
      const uploaded = await Promise.all(files.map(readFileAsDataUrl))
      updateStageTemplate((template) => ({
        ...template,
        images: [...template.images, ...uploaded],
      }))
      notify('success', `已添加 ${uploaded.length} 张图片`)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      event.target.value = ''
    }
  }

  function insertVariable(key: string) {
    if (!settings) return
    const currentText = settings[selectedStage].text_template
    const textarea = templateTextareaRef.current
    const token = `<${key}>`
    const selectionStart = textarea?.selectionStart ?? currentText.length
    const selectionEnd = textarea?.selectionEnd ?? currentText.length
    updateStageTemplate((template) => ({
      ...template,
      text_template: `${currentText.slice(0, selectionStart)}${token}${currentText.slice(selectionEnd)}`,
    }))
    window.requestAnimationFrame(() => {
      const nextCaret = selectionStart + token.length
      templateTextareaRef.current?.focus()
      templateTextareaRef.current?.setSelectionRange(nextCaret, nextCaret)
    })
  }

  if (loading && !settings) {
    return <EmptyPanel icon={<Spinner />} text="正在加载用户回链设置" />
  }

  if (!settings) {
    return (
      <div className="workspace">
        <header className="page-head">
          <div>
            <h1>用户回链</h1>
            <p>配置进入队列、发稿成功、拒稿三个阶段的用户通知。</p>
          </div>
          <Button size="sm" variant="secondary" onClick={() => void loadSettings()}>
            <RefreshCcw size={16} />
            重试
          </Button>
        </header>
        <Card className="panel-card">
          <Card.Content>
            <EmptyPanel icon={<Send size={28} />} text="当前没有可用的回链设置数据" />
          </Card.Content>
        </Card>
      </div>
    )
  }

  const activeStage = NOTIFICATION_STAGE_OPTIONS.find((item) => item.value === selectedStage)!
  const activeTemplate = settings[selectedStage]
  const groupOptions = settings.available_groups.map((group) => ({ value: group, label: group }))
  const menuOptions = [
    {
      value: 'stages' as const,
      label: '阶段模板',
      icon: <Send size={16} />,
    },
    {
      value: 'webhooks' as const,
      label: 'Webhook 标签',
      icon: <Zap size={16} />,
    },
    {
      value: 'variables' as const,
      label: '变量说明',
      icon: <HelpCircle size={16} />,
    },
  ]

  function renderGroupCard(extra?: ReactNode) {
    return (
      <Card className="panel-card">
        <Card.Content>
          <div className="settings-toolbar">
            <div className="field-stack">
              <span className="field-label">群组</span>
              <HeroSelect
                className="control settings-group-select"
                ariaLabel="选择群组"
                selectedKey={selectedGroup}
                options={groupOptions}
                onSelect={(value) => {
                  setSelectedGroup(value)
                  void loadSettings(value)
                }}
              />
            </div>
            {extra}
          </div>
        </Card.Content>
      </Card>
    )
  }

  return (
    <div className="workspace settings-workspace">
      <header className="page-head">
        <div>
          <h1>用户回链</h1>
          <p>将阶段模板、Webhook 标签和变量说明拆成独立菜单，方便单独维护。</p>
        </div>
        <div className="head-actions">
          <Button
            size="sm"
            variant="secondary"
            isDisabled={loading || saving}
            onClick={() => void loadSettings(selectedGroup)}
          >
            <RefreshCcw size={16} />
            刷新
          </Button>
          <Button size="sm" isDisabled={loading || saving} onClick={() => void saveSettings()}>
            {saving ? <Spinner size="sm" /> : <Check size={16} />}
            保存
          </Button>
        </div>
      </header>

      <section className="settings-layout">
        <SettingsMenuCard
          title="回链菜单"
          options={menuOptions}
          activeKey={selectedMenu}
          onSelect={setSelectedMenu}
        />

        <div className="settings-content-stack">
          {selectedMenu === 'stages' ? (
            <>
              {renderGroupCard()}

              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>阶段选择</Card.Title>
                </Card.Header>
                <Card.Content>
                  <div className="settings-stage-grid stage-selector-grid">
                    {NOTIFICATION_STAGE_OPTIONS.map((stage) => {
                      const template = settings[stage.value]
                      return (
                        <button
                          key={stage.value}
                          type="button"
                          className={`settings-stage-button settings-stage-card ${
                            selectedStage === stage.value ? 'is-active' : ''
                          }`}
                          onClick={() => setSelectedStage(stage.value)}
                        >
                          <span className="settings-stage-icon">{stage.icon}</span>
                          <strong>{stage.label}</strong>
                          <div className="stage-card-metrics">
                            <span>{template.tags.length} 个标签</span>
                            <span>{template.images.length} 张图片</span>
                          </div>
                          <NotificationStageCardPreview images={template.images} />
                        </button>
                      )
                    })}
                  </div>
                </Card.Content>
              </Card>

              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>{activeStage.label}</Card.Title>
                </Card.Header>
                <Card.Content>
                  <div className="settings-panel">
                    <div className="settings-inline-grid">
                      <div className="field-stack settings-switch-stack">
                        <span className="field-label">当前阶段</span>
                        <span className="field-hint">{activeStage.label}</span>
                      </div>
                      <div className="field-stack settings-switch-stack">
                        <span className="field-label">启用通知</span>
                        <Switch
                          isSelected={activeTemplate.enabled}
                          onChange={(value) =>
                            updateStageTemplate((template) => ({ ...template, enabled: value }))
                          }
                          size="sm"
                        >
                          {activeTemplate.enabled ? '已启用' : '已关闭'}
                        </Switch>
                      </div>
                      <div className="field-stack settings-switch-stack">
                        <span className="field-label">自动带上稿件标签</span>
                        <Switch
                          isSelected={activeTemplate.include_post_tags}
                          onChange={(value) =>
                            updateStageTemplate((template) => ({
                              ...template,
                              include_post_tags: value,
                            }))
                          }
                          size="sm"
                        >
                          {activeTemplate.include_post_tags ? '已启用' : '已关闭'}
                        </Switch>
                      </div>
                    </div>

                    <div className="field-stack">
                      <span className="field-label">通知文案</span>
                      <TextArea
                        ref={templateTextareaRef}
                        className="settings-template-textarea"
                        placeholder="#<code> 已处理"
                        value={activeTemplate.text_template}
                        onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                          updateStageTemplate((template) => ({
                            ...template,
                            text_template: event.target.value,
                          }))
                        }
                      />
                    </div>

                    <div className="field-stack">
                      <div className="settings-section-head">
                        <span className="field-label">可插入变量</span>
                        <span className="field-hint">点击后会在当前光标处插入变量。</span>
                      </div>
                      <div className="settings-variable-grid">
                        {settings.variables.map((variable) => (
                          <button
                            key={variable.key}
                            type="button"
                            className="settings-variable-button"
                            onClick={() => insertVariable(variable.key)}
                          >
                            <strong>{variable.label}</strong>
                            <code>{variable.example}</code>
                          </button>
                        ))}
                      </div>
                    </div>

                    <div className="settings-subsection">
                      <div className="settings-section-head">
                        <span className="field-label">阶段图片</span>
                        <div className="settings-image-actions">
                          <Button
                            size="sm"
                            variant="secondary"
                            onClick={() => addStageListItem('images')}
                          >
                            新增图片
                          </Button>
                          <Button
                            size="sm"
                            variant="secondary"
                            onClick={() => uploadInputRef.current?.click()}
                          >
                            上传图片
                          </Button>
                          <input
                            ref={uploadInputRef}
                            className="sr-only"
                            type="file"
                            accept="image/*"
                            multiple
                            onChange={uploadImages}
                          />
                        </div>
                      </div>
                      {activeTemplate.images.length ? (
                        <div className="settings-image-list">
                          {activeTemplate.images.map((image: string, index: number) => {
                            const previewable = isPreviewableImageSource(image)
                            return (
                              <div
                                key={`stage-image-${selectedStage}-${index}`}
                                className="settings-image-item"
                              >
                                <div className="settings-image-row">
                                  <Input
                                    className="settings-image-input"
                                    placeholder="https://example.com/success.png 或 data:image/..."
                                    value={image}
                                    onChange={(event) =>
                                      updateStageList('images', index, event.target.value)
                                    }
                                  />
                                  <Button
                                    size="sm"
                                    variant="tertiary"
                                    isIconOnly
                                    aria-label={`删除图片 ${index + 1}`}
                                    onClick={() => removeStageListItem('images', index)}
                                  >
                                    <Trash2 size={16} />
                                  </Button>
                                </div>
                                <div className="settings-image-meta">
                                  <span className="field-hint">
                                    {buildReplyImageLabel(image, index)}
                                  </span>
                                </div>
                                {previewable ? (
                                  <div className="settings-image-preview-frame">
                                    <img
                                      className="settings-image-preview"
                                      src={image.trim()}
                                      alt={`阶段图片预览 ${index + 1}`}
                                    />
                                  </div>
                                ) : null}
                              </div>
                            )
                          })}
                        </div>
                      ) : (
                        <div className="settings-empty">
                          <MessageSquare size={20} />
                          <span>当前阶段未附带图片。</span>
                        </div>
                      )}
                    </div>
                  </div>
                </Card.Content>
              </Card>
            </>
          ) : null}

          {selectedMenu === 'webhooks' ? (
            <>
              {renderGroupCard()}

              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>Webhook 标签映射</Card.Title>
                </Card.Header>
                <Card.Content>
                  <MappingListEditor
                    label="Webhook 标签"
                    leftPlaceholder="Webhook"
                    rightPlaceholder="默认标签"
                    entries={settings.webhook_tag_map}
                    onAdd={() => addMapping('webhook_tag_map')}
                    onChange={(index, key, value) =>
                      updateMapping('webhook_tag_map', index, key, value)
                    }
                    onRemove={(index) => removeMapping('webhook_tag_map', index)}
                  />
                </Card.Content>
              </Card>
            </>
          ) : null}

          {selectedMenu === 'variables' ? (
            <>
              {renderGroupCard()}

              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>变量说明</Card.Title>
                </Card.Header>
                <Card.Content>
                  <div className="settings-variable-list">
                    {settings.variables.map((variable) => (
                      <div key={variable.key} className="settings-variable-card">
                        <div className="settings-variable-meta">
                          <strong>{variable.label}</strong>
                          <code>{variable.example}</code>
                        </div>
                        <p>{variable.description}</p>
                      </div>
                    ))}
                  </div>
                </Card.Content>
              </Card>
            </>
          ) : null}
        </div>
      </section>
    </div>
  )
}

function StatsView({ notify }: { notify: (kind: ToastKind, text: string) => void }) {
  const [stats, setStats] = useState<StatsResponse | null>(null)
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    loadStats()
  }, [])

  async function loadStats() {
    setLoading(true)
    try {
      setStats(await api<StatsResponse>('/api/stats'))
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  if (loading && !stats) return <EmptyPanel icon={<Spinner />} text="正在加载统计" />
  if (!stats) return <EmptyPanel icon={<BarChart3 size={28} />} text="暂无统计数据" />

  const maxDaily = Math.max(1, ...stats.daily_trend.map((item) => item.submitted))
  const maxHourly = Math.max(1, ...stats.hourly_distribution.map((item) => item.count))

  return (
    <div className="workspace">
      <header className="page-head">
        <div>
          <h1>运行统计</h1>
          <p>当前状态快照</p>
        </div>
        <Button size="sm" variant="secondary" onClick={loadStats}>
          <RefreshCcw size={16} />
          刷新
        </Button>
      </header>

      <section className="metrics" aria-label="运行指标">
        <Metric label="待审核" value={stats.pending_count} tone="warn" icon={<Clock3 size={18} />} />
        <Metric label="今日投稿" value={stats.today_count} tone="good" icon={<FileText size={18} />} />
        <Metric label="总投稿" value={stats.total_count} tone="neutral" icon={<Inbox size={18} />} />
        <Metric label="平均审核" value={formatDuration(stats.avg_review_time_ms)} tone="neutral" icon={<UserRound size={18} />} />
      </section>

      <section className="stats-grid">
        <Card className="panel-card">
          <Card.Header>
            <Card.Title>状态分布</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="stage-list">
              {Object.entries(stats.stage_breakdown).map(([stage, count]) => (
                <div key={stage}>
                  <span>{STAGE_LABELS[stage] ?? stage}</span>
                  <strong>{count}</strong>
                </div>
              ))}
            </div>
          </Card.Content>
        </Card>
        <Card className="panel-card">
          <Card.Header>
            <Card.Title>近 14 天</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="bar-list">
              {stats.daily_trend.map((item) => (
                <div key={item.date} className="bar-row">
                  <span>{item.date.slice(5)}</span>
                  <div>
                    <i style={{ width: `${(item.submitted / maxDaily) * 100}%` }} />
                  </div>
                  <strong>{item.submitted}</strong>
                </div>
              ))}
            </div>
          </Card.Content>
        </Card>
        <Card className="panel-card wide">
          <Card.Header>
            <Card.Title>小时分布</Card.Title>
          </Card.Header>
          <Card.Content>
            <div className="hour-grid">
              {stats.hourly_distribution.map((item) => (
                <div key={item.hour} title={`${item.hour}:00 ${item.count} 条`}>
                  <span style={{ opacity: 0.18 + (item.count / maxHourly) * 0.82 }} />
                  <small>{item.hour}</small>
                </div>
              ))}
            </div>
          </Card.Content>
        </Card>
      </section>
    </div>
  )
}

function HeroSelect({
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

function DelayField({
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

function Metric({
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

function StageChip({ stage }: { stage: string }) {
  const color =
    stage === 'review_pending'
      ? 'warning'
      : stage === 'failed' || stage === 'rejected'
        ? 'danger'
        : stage === 'sent'
          ? 'success'
          : 'default'
  return (
    <Chip color={color} variant="soft" size="sm">
      {STAGE_LABELS[stage] ?? stage}
    </Chip>
  )
}

function quickActionVariant(action: string): ComponentProps<typeof Button>['variant'] {
  if (action === 'reject' || action === 'delete' || action === 'blacklist') return 'danger-soft'
  if (action === 'approve') return undefined
  if (action === 'immediate') return 'secondary'
  return 'secondary'
}

function cardActionLabel(action: string) {
  if (action === 'skip') return '否'
  if (action === 'immediate') return '立即'
  return ACTION_LABELS[action] ?? action
}

function quickActionIcon(action: string) {
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

function EmptyPanel({ icon, text }: { icon: React.ReactNode; text: string }) {
  return (
    <EmptyState className="empty-state">
      <div className="empty-icon">{icon}</div>
      <span>{text}</span>
    </EmptyState>
  )
}

function showToast(kind: ToastKind, text: string) {
  if (kind === 'success') {
    toast.success(text)
  } else if (kind === 'error') {
    toast.danger(text)
  } else {
    toast.info(text)
  }
}

function useMediaQuery(query: string) {
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

function useMasonryLayout(dependencies: React.DependencyList) {
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

function buildReplyImageLabel(value: string, index: number) {
  const trimmed = value.trim()
  if (!trimmed) return `图片 ${index + 1}：未填写地址`
  if (trimmed.startsWith('data:image/')) return `图片 ${index + 1}：已上传`
  return trimmed.length > 72 ? `${trimmed.slice(0, 72)}...` : trimmed
}

function isPreviewableImageSource(value: string) {
  const trimmed = value.trim().toLowerCase()
  return (
    trimmed.startsWith('data:image/') ||
    trimmed.startsWith('http://') ||
    trimmed.startsWith('https://')
  )
}

function readFileAsDataUrl(file: File): Promise<string> {
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

function buildPostParams({
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

function buildActionPayload(action: string, text: string, delayMs: number) {
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

function formatDateTime(ms: number) {
  return new Date(ms).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function formatDuration(ms: number | null) {
  if (!ms) return '-'
  const minutes = Math.round(ms / 60000)
  if (minutes < 60) return `${minutes} 分钟`
  return `${Math.round(minutes / 60)} 小时`
}

export default App

