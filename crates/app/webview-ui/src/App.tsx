import {
  type ComponentProps,
  type CSSProperties,
  type FormEvent,
  type Key,
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
  Toast,
  ToggleButton,
  ToggleButtonGroup,
  toast,
} from '@heroui/react'
import {
  AlertCircle,
  BarChart3,
  Ban,
  Bookmark,
  Check,
  CheckCircle2,
  Clock3,
  Eye,
  FileClock,
  FileSearch,
  FileText,
  HelpCircle,
  History,
  Inbox,
  LayoutGrid,
  LayoutDashboard,
  List,
  LogOut,
  MessageSquare,
  MoreHorizontal,
  PanelRightOpen,
  RefreshCcw,
  Search,
  Send,
  ShieldCheck,
  Sparkles,
  Trash2,
  UserRound,
  Users,
  X,
  Zap,
} from 'lucide-react'
import { api } from './api/client'
import {
  ACTION_LABELS,
  AuditListResponse,
  BlacklistItem,
  BlacklistListResponse,
  FailureItem,
  FailureListResponse,
  GroupHealthItem,
  GroupHealthResponse,
  ListPostsResponse,
  ListReviewIdsResponse,
  MeResponse,
  PostCollectionResponse,
  PostDetail,
  PostItem,
  SavedFilterListResponse,
  SavedFilterPreset,
  SimilarPostResponse,
  Stage,
  StatsResponse,
  STAGE_LABELS,
} from './api/types'

type ViewKey = 'overview' | 'review' | 'failures' | 'blacklist' | 'audit' | 'stats'
type PostViewMode = 'cards' | 'list'
type ToastKind = 'info' | 'success' | 'error'
type SortOrder = 'asc' | 'desc'
type SelectOption<T extends string = string> = { value: T; label: string }

type ReviewFilters = {
  stage: Stage
  keyword: string
  groupId: string
  sortBy: string
  sortOrder: SortOrder
  onlyError: boolean
  onlyActionable: boolean
  page: number
  pageSize: number
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

const STAGE_OPTIONS: Array<SelectOption<Stage>> = [
  { value: '__active__', label: '全部活跃' },
  { value: '', label: '全部' },
  { value: 'review_pending', label: '待审核' },
  { value: 'reviewed', label: '已审核' },
  { value: 'scheduled', label: '已排队' },
  { value: 'sending', label: '发送中' },
  { value: 'sent', label: '已发送' },
  { value: 'rejected', label: '已拒绝' },
  { value: 'deleted', label: '已删除' },
  { value: 'withdrawn', label: '已撤回' },
  { value: 'skipped', label: '已跳过' },
  { value: 'manual', label: '人工处理' },
  { value: 'failed', label: '失败' },
]

const SORT_OPTIONS: Array<SelectOption> = [
  { value: 'created_at:desc', label: '最新优先' },
  { value: 'created_at:asc', label: '最早优先' },
  { value: 'code:desc', label: '编号优先' },
  { value: 'stage:asc', label: '状态排序' },
  { value: 'group_id:asc', label: '分组排序' },
]

const PAGE_SIZES = [20, 50, 100, 200]
const ACTIVE_EXCLUDED = new Set(['rejected', 'deleted', 'withdrawn', 'skipped', 'failed'])
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
  'expand_audit',
  'show',
  'comment',
  'reply',
  'blacklist',
  'quick_reply',
  'merge',
]

function App() {
  const [me, setMe] = useState<MeResponse | null>(null)
  const [authChecked, setAuthChecked] = useState(false)
  const [view, setView] = useState<ViewKey>('overview')

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
      <div className="admin-shell">
        <aside className="admin-sidebar">
          <Brand large />
          <SidebarNav current={view} onChange={setView} />
          <Card className="account-card" variant="secondary">
            <Card.Content>
              <div className="account-name">{me.username}</div>
              <div className="account-role">
                {me.role === 'global_admin' ? '全局管理员' : me.groups.join('、')}
              </div>
              <Button size="sm" variant="secondary" fullWidth onClick={logout}>
                <LogOut size={16} />
                退出
              </Button>
            </Card.Content>
          </Card>
        </aside>

        <main className="admin-main">
          {view === 'overview' && <OverviewView onJump={setView} notify={notify} />}
          {view === 'review' && <ReviewView notify={notify} />}
          {view === 'failures' && <FailuresView notify={notify} />}
          {view === 'blacklist' && <BlacklistView notify={notify} />}
          {view === 'audit' && <AuditView notify={notify} />}
          {view === 'stats' && <StatsView notify={notify} />}
        </main>
      </div>
      <MobileTabbar current={view} onChange={setView} />
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

function Brand({ large = false }: { large?: boolean }) {
  return (
    <div className={large ? 'brand brand-large' : 'brand'}>
      <div>
        <strong>OQQWall</strong>
        <span>稿件管理后台</span>
      </div>
    </div>
  )
}

function SidebarNav({
  current,
  onChange,
}: {
  current: ViewKey
  onChange: (value: ViewKey) => void
}) {
  const items: Array<{ key: ViewKey; label: string; icon: React.ReactNode }> = [
    { key: 'overview', label: '概览', icon: <LayoutDashboard size={18} /> },
    { key: 'review', label: '主操作台', icon: <Eye size={18} /> },
    { key: 'failures', label: '失败中心', icon: <AlertCircle size={18} /> },
    { key: 'blacklist', label: '黑名单', icon: <Ban size={18} /> },
    { key: 'audit', label: '操作审计', icon: <History size={18} /> },
    { key: 'stats', label: '运行统计', icon: <BarChart3 size={18} /> },
  ]

  return (
    <nav className="nav" aria-label="后台导航">
      {items.map((item) => (
        <Button
          key={item.key}
          className="nav-button"
          variant={current === item.key ? 'primary' : 'tertiary'}
          fullWidth
          onClick={() => onChange(item.key)}
        >
          {item.icon}
          {item.label}
        </Button>
      ))}
    </nav>
  )
}

function MobileTabbar({
  current,
  onChange,
}: {
  current: ViewKey
  onChange: (value: ViewKey) => void
}) {
  const items: Array<{ key: ViewKey; label: string; icon: React.ReactNode }> = [
    { key: 'overview', label: '概览', icon: <LayoutDashboard size={18} /> },
    { key: 'review', label: '操作', icon: <Eye size={18} /> },
    { key: 'failures', label: '失败', icon: <AlertCircle size={18} /> },
    { key: 'blacklist', label: '黑名单', icon: <Ban size={18} /> },
    { key: 'audit', label: '更多', icon: <History size={18} /> },
  ]

  return (
    <nav className="mobile-tabbar" aria-label="移动端底部导航">
      {items.map((item) => (
        <button
          key={item.key}
          type="button"
          className={current === item.key ? 'mobile-tab active' : 'mobile-tab'}
          onClick={() => onChange(item.key)}
        >
          {item.icon}
          <span>{item.label}</span>
        </button>
      ))}
    </nav>
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

function OverviewView({
  onJump,
  notify,
}: {
  onJump: (value: ViewKey) => void
  notify: (kind: ToastKind, text: string) => void
}) {
  const [stats, setStats] = useState<StatsResponse | null>(null)
  const [groups, setGroups] = useState<GroupHealthItem[]>([])
  const [failures, setFailures] = useState<FailureItem[]>([])
  const [latestPosts, setLatestPosts] = useState<PostItem[]>([])
  const [loading, setLoading] = useState(false)

  async function loadOverview() {
    setLoading(true)
    try {
      const [statsResult, groupsResult, failuresResult] = await Promise.all([
        api<StatsResponse>('/api/stats'),
        api<GroupHealthResponse>('/api/overview/groups'),
        api<FailureListResponse>('/api/failures?limit=6'),
      ])
      setStats(statsResult)
      setGroups(groupsResult.items)
      setFailures(failuresResult.items)
      const latestPostsResult = await api<ListPostsResponse>('/api/posts?active_only=true&limit=4&sort_by=created_at&sort_order=desc')
      setLatestPosts(latestPostsResult.items)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void loadOverview()
  }, [])

  return (
    <div className="workspace">
      <header className="page-head">
        <div>
          <h1>后台概览</h1>
          <p>运营视角下的稿件健康、处理压力与异常告警</p>
        </div>
        <Button size="sm" variant="secondary" onClick={loadOverview}>
          <RefreshCcw size={16} />
          刷新概览
        </Button>
      </header>

      {loading && !stats ? (
        <LoadingPanel text="正在加载概览" />
      ) : !stats ? (
        <EmptyPanel icon={<LayoutDashboard size={28} />} text="暂无概览数据" />
      ) : (
        <>
          <section className="metrics overview-metrics">
            <Metric label="待审核" value={stats.pending_count} tone="warn" icon={<Clock3 size={18} />} />
            <Metric label="今日投稿" value={stats.today_count} tone="good" icon={<Inbox size={18} />} />
            <Metric label="异常告警" value={stats.error_count} tone="bad" icon={<AlertCircle size={18} />} />
            <Metric label="可操作" value={stats.actionable_count} tone="neutral" icon={<Sparkles size={18} />} />
          </section>

          <section className="overview-grid">
            <Card className="panel-card wide-card">
              <Card.Header>
                <Card.Title>快捷入口</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="quick-entry-grid">
                  <QuickEntryCard title="主操作台" text="直接进入高频审核工作区" icon={<Eye size={18} />} onClick={() => onJump('review')} />
                  <QuickEntryCard title="失败中心" text="查看渲染、发布、状态异常" icon={<AlertCircle size={18} />} onClick={() => onJump('failures')} />
                  <QuickEntryCard title="黑名单" text="维护风险投稿人与原因" icon={<Ban size={18} />} onClick={() => onJump('blacklist')} />
                  <QuickEntryCard title="操作审计" text="回看人工管理动作" icon={<History size={18} />} onClick={() => onJump('audit')} />
                </div>
              </Card.Content>
            </Card>

            <Card className="panel-card">
              <Card.Header>
                <Card.Title>组别健康度</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="list-stack">
                  {groups.slice(0, 6).map((item) => (
                    <div key={item.group_id} className="group-health-row">
                      <div>
                        <strong>{item.group_id}</strong>
                        <span>
                          待审 {item.pending_count} · 异常 {item.error_count} · 已发 {item.sent_count}
                        </span>
                      </div>
                      <Chip size="sm" variant="soft">
                        {formatDuration(item.avg_review_time_ms)}
                      </Chip>
                    </div>
                  ))}
                </div>
              </Card.Content>
            </Card>

            <Card className="panel-card">
              <Card.Header>
                <Card.Title>最近告警</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="list-stack">
                  {failures.length ? (
                    failures.map((item) => (
                      <div key={`${item.post_id}-${item.source}`} className="failure-row">
                        <div>
                          <strong>#{item.review_code ?? '-'}</strong>
                          <span>{item.group_id} · {item.source}</span>
                        </div>
                        <p>{item.error}</p>
                      </div>
                    ))
                  ) : (
                    <EmptyInline text="当前没有异常稿件" />
                  )}
                </div>
              </Card.Content>
            </Card>

            <Card className="panel-card wide-card">
              <Card.Header>
                <Card.Title>最新稿件预览</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="overview-preview-grid">
                  {latestPosts.length ? (
                    latestPosts.map((post) => (
                      <article key={post.post_id} className="overview-preview-card">
                        {post.preview_image_url ? (
                          <img className="overview-preview-image" src={post.preview_image_url} alt="稿件预览" />
                        ) : (
                          <div className="overview-preview-fallback">
                            <FileImageIcon />
                          </div>
                        )}
                        <div className="overview-preview-meta">
                          <strong>#{post.internal_code ?? post.external_code ?? '-'}</strong>
                          <span>{post.group_id}</span>
                          <p>{post.preview_text || '该稿件暂无文本预览'}</p>
                        </div>
                      </article>
                    ))
                  ) : (
                    <EmptyInline text="当前没有可展示的最新稿件" />
                  )}
                </div>
              </Card.Content>
            </Card>

            <Card className="panel-card wide-card">
              <Card.Header>
                <Card.Title>阶段分布</Card.Title>
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
          </section>
        </>
      )}
    </div>
  )
}

function ReviewView({ notify }: { notify: (kind: ToastKind, text: string) => void }) {
  const [posts, setPosts] = useState<PostItem[]>([])
  const [total, setTotal] = useState(0)
  const [loading, setLoading] = useState(false)
  const [actionLoading, setActionLoading] = useState(false)
  const [stage, setStage] = useState<Stage>('__active__')
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
  const [postView, setPostView] = useState<PostViewMode>('list')
  const [savedFilters, setSavedFilters] = useState<SavedFilterPreset[]>([])
  const [recentOps, setRecentOps] = useState<Array<{ time: number; text: string }>>([])
  const [presetName, setPresetName] = useState('')
  const compactDetail = useMediaQuery('(max-width: 980px)')

  const groups = useMemo(() => [...new Set(posts.map((post) => post.group_id))].sort(), [posts])
  const visiblePosts = useMemo(() => {
    let out = posts
    if (stage === '__active__') out = out.filter((post) => !ACTIVE_EXCLUDED.has(post.stage))
    if (onlyError) out = out.filter((post) => !!post.last_error)
    if (onlyActionable) out = out.filter((post) => !!post.review_id)
    return out
  }, [posts, stage, onlyError, onlyActionable])
  const selectableIds = useMemo(
    () => visiblePosts.map((post) => post.review_id).filter(Boolean) as string[],
    [visiblePosts],
  )
  const totalPages = Math.max(1, Math.ceil(total / pageSize))
  const currentSelectedCount = selectAllTotal ?? selected.length
  const detailIndex = detail ? visiblePosts.findIndex((post) => post.post_id === detail.post_id) : -1
  const showDetailPanel = !compactDetail && (!!detail || detailLoading)

  useEffect(() => {
    void loadPosts()
  }, [stage, groupId, sortBy, sortOrder, page, pageSize, onlyError, onlyActionable])

  useEffect(() => {
    void loadSavedFilters()
  }, [])

  useEffect(() => {
    if (!autoRefresh) return
    const id = window.setInterval(() => void loadPosts(), 30000)
    return () => window.clearInterval(id)
  }, [autoRefresh, stage, groupId, sortBy, sortOrder, page, pageSize, keyword, onlyError, onlyActionable])

  useEffect(() => {
    setActionText('')
  }, [batchAction])

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

  async function loadSavedFilters() {
    try {
      const result = await api<SavedFilterListResponse>('/api/filter-presets')
      setSavedFilters(result.items)
    } catch {
      // ignore
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
      stage: '__active__' as Stage,
      keyword: '',
      groupId: '',
      sortBy: 'created_at',
      sortOrder: 'desc' as SortOrder,
      page: 0,
      onlyError: false,
      onlyActionable: false,
    }
    setStage('__active__')
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
        body: JSON.stringify(buildActionPayload(action, textOverride ?? '', actionDelay)),
      })
      pushRecentOp(`执行 ${ACTION_LABELS[action] ?? action}`)
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
      pushRecentOp(`批量执行 ${ACTION_LABELS[batchAction] ?? batchAction}（${currentSelectedCount} 条）`)
      notify('success', `批量执行完成：${ACTION_LABELS[batchAction] ?? batchAction}`)
      setSelected([])
      setSelectAllTotal(null)
      setActionText('')
      await loadPosts({ resetSelection: true })
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setActionLoading(false)
    }
  }

  async function saveCurrentFilter() {
    if (!presetName.trim()) {
      notify('info', '请先填写筛选器名称')
      return
    }
    try {
      await api('/api/filter-presets', {
        method: 'POST',
        body: JSON.stringify({
          name: presetName.trim(),
          query: {
            stage,
            keyword,
            group_id: groupId,
            sort_by: sortBy,
            sort_order: sortOrder,
            only_error: onlyError,
            only_actionable: onlyActionable,
            page_size: pageSize,
          },
        }),
      })
      setPresetName('')
      await loadSavedFilters()
      notify('success', '筛选器已保存')
    } catch (error) {
      notify('error', (error as Error).message)
    }
  }

  function pushRecentOp(text: string) {
    setRecentOps((prev) => [{ time: Date.now(), text }, ...prev].slice(0, 8))
  }

  function applyPreset(preset: SavedFilterPreset) {
    setStage((preset.query.stage as Stage) || '__active__')
    setKeyword(preset.query.keyword || '')
    setGroupId(preset.query.group_id || '')
    setSortBy(preset.query.sort_by || 'created_at')
    setSortOrder((preset.query.sort_order as SortOrder) || 'desc')
    setOnlyError(preset.query.only_error)
    setOnlyActionable(preset.query.only_actionable)
    setPageSize(preset.query.page_size || 50)
    setPage(0)
    setSelected([])
    setSelectAllTotal(null)
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
          <Button size="sm" variant="secondary" onClick={() => void loadPosts()}>
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

      <Card className="control-card">
        <Card.Content>
          <div className="toolbar-grid">
            <HeroSelect
              className="control"
              ariaLabel="状态筛选"
              selectedKey={stage}
              options={STAGE_OPTIONS}
              onSelect={(value) => {
                setStage(value as Stage)
                setPage(0)
              }}
            />
            <HeroSelect
              className="control"
              ariaLabel="分组筛选"
              selectedKey={groupId}
              options={[{ value: '', label: '全部分组' }, ...groups.map((group) => ({ value: group, label: group }))]}
              onSelect={(value) => {
                setGroupId(value)
                setPage(0)
              }}
            />
            <HeroSelect
              className="control"
              ariaLabel="排序"
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
              placeholder="搜索编号、投稿人、内容、错误"
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
            <Checkbox isSelected={onlyActionable} onChange={setOnlyActionable}>
              可操作
            </Checkbox>
            <Checkbox isSelected={onlyError} onChange={setOnlyError}>
              异常
            </Checkbox>
            <Input
              className="preset-input"
              placeholder="保存当前筛选为..."
              value={presetName}
              onChange={(event) => setPresetName(event.target.value)}
            />
            <Button size="sm" variant="secondary" onClick={saveCurrentFilter}>
              <Bookmark size={16} />
              保存筛选
            </Button>
          </div>
        </Card.Content>
      </Card>

      <Card className="control-card">
        <Card.Content>
          <div className="batch-row">
            <div className="batch-actions">
              <Button size="sm" variant="secondary" onClick={togglePageSelection}>
                {selectableIds.length > 0 && selectableIds.every((id) => selected.includes(id))
                  ? '取消本页'
                  : '选择本页'}
              </Button>
              <Button size="sm" variant="secondary" onClick={selectAcrossPages}>
                选择当前筛选
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
                ariaLabel="批量动作"
                selectedKey={batchAction}
                options={BATCH_ACTIONS.map((action) => ({ value: action, label: ACTION_LABELS[action] ?? action }))}
                onSelect={setBatchAction}
              />
              {batchAction === 'defer' && (
                <DelayField value={actionDelay} onChange={setActionDelay} className="delay-field" />
              )}
              {(batchAction === 'reject' || batchAction === 'delete') && (
                <Input
                  className="action-text"
                  placeholder="统一理由，可留空"
                  value={actionText}
                  onChange={(event) => setActionText(event.target.value)}
                />
              )}
              <Button size="sm" isDisabled={!selected.length || actionLoading} onClick={runBatch}>
                {actionLoading ? <Spinner size="sm" /> : <Check size={16} />}
                批量执行
              </Button>
            </div>
          </div>
        </Card.Content>
      </Card>

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
                <LoadingPanel text="正在加载稿件" />
              ) : visiblePosts.length ? (
                postView === 'cards' ? (
                  <PostCards
                    posts={visiblePosts}
                    activePostId={detail?.post_id ?? null}
                    selected={selected}
                    selectAllTotal={selectAllTotal}
                    actionLoading={actionLoading}
                    onToggle={toggleOne}
                    onOpen={openDetail}
                    onAction={(reviewId, action, text) => void runAction(reviewId, action, text)}
                  />
                ) : (
                  <PostTable
                    posts={visiblePosts}
                    activePostId={detail?.post_id ?? null}
                    selected={selected}
                    selectAllTotal={selectAllTotal}
                    actionLoading={actionLoading}
                    onToggle={toggleOne}
                    onOpen={openDetail}
                    onAction={(reviewId, action, text) => void runAction(reviewId, action, text)}
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
              savedFilters={savedFilters}
              recentOps={recentOps}
              onApplyPreset={applyPreset}
              onClose={() => setDetail(null)}
              onRefresh={refreshDetail}
              onTextChange={setActionText}
              onDelayChange={setActionDelay}
              onAction={(action) => detail?.review_id && void runAction(detail.review_id, action, actionText)}
              onPrev={() => detailIndex > 0 && void openDetail(visiblePosts[detailIndex - 1].post_id)}
              onNext={() =>
                detailIndex >= 0 &&
                detailIndex < visiblePosts.length - 1 &&
                void openDetail(visiblePosts[detailIndex + 1].post_id)
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
          onClose={() => setDetail(null)}
          onRefresh={refreshDetail}
          onTextChange={setActionText}
          onDelayChange={setActionDelay}
          onAction={(action) => detail?.review_id && void runAction(detail.review_id, action, actionText)}
          onPrev={() => detailIndex > 0 && void openDetail(visiblePosts[detailIndex - 1].post_id)}
          onNext={() =>
            detailIndex >= 0 &&
            detailIndex < visiblePosts.length - 1 &&
            void openDetail(visiblePosts[detailIndex + 1].post_id)
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
  onToggle,
  onOpen,
  onAction,
}: {
  posts: PostItem[]
  activePostId: string | null
  selected: string[]
  selectAllTotal: number | null
  actionLoading: boolean
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
      <Card className={active ? 'post-card active' : 'post-card'} variant="secondary">
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
            {post.review_id && (
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
            {post.preview_text && imageUrls.length === 0 && <span className="post-card-preview">{post.preview_text}</span>}
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
              placeholder="评论或拒绝/删除/拉黑原因"
              value={note}
              onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) => onNoteChange(event.target.value)}
            />
          )}
          {post.last_error && <div className="post-card-error">{post.last_error}</div>}
        </Card.Content>
        <Card.Footer className="post-card-footer">
          {post.review_id ? (
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
  onToggle,
  onOpen,
  onAction,
}: {
  posts: PostItem[]
  activePostId: string | null
  selected: string[]
  selectAllTotal: number | null
  actionLoading: boolean
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
                    {post.review_id ? (
                      <button
                        type="button"
                        className={selectAllTotal !== null || selected.includes(post.review_id) ? 'list-checkbox checked' : 'list-checkbox'}
                        aria-label={`选择 ${post.internal_code ?? post.external_code ?? post.post_id}`}
                        aria-pressed={selectAllTotal !== null || selected.includes(post.review_id)}
                        onClick={() => onToggle(post.review_id!, !(selectAllTotal !== null || selected.includes(post.review_id!)))}
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
                    {post.preview_image_url ? <img className="list-preview-thumb" src={post.preview_image_url} alt="稿件缩略图" /> : null}
                    <div className="list-preview-text">
                      <div className="preview">{post.preview_text || (post.preview_image_url ? '[图片]' : '-')}</div>
                      <span>
                        {post.group_id} · {post.sender_id ?? '未知投稿人'}
                      </span>
                    </div>
                  </div>
                </td>
                <td>{formatDateTime(post.created_at_ms)}</td>
                <td>
                  {post.review_id ? (
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
  if (action === 'reject' || action === 'delete') {
    return window.prompt('可填写原因，留空将直接执行') ?? null
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
  hasNext: boolean
  savedFilters?: SavedFilterPreset[]
  recentOps?: Array<{ time: number; text: string }>
  onApplyPreset?: (preset: SavedFilterPreset) => void
  onRefresh: () => void
  onTextChange: (value: string) => void
  onDelayChange: (value: number) => void
  onAction: (action: string) => void
  onPrev: () => void
  onNext: () => void
}

function InlineDetailPanel(props: DetailContentProps & { onClose: () => void }) {
  const { detail, loading, onClose, savedFilters = [], recentOps = [], onApplyPreset } = props

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
          <>
            <DetailContent {...props} />
            <section className="detail-side-panels">
              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>已保存筛选</Card.Title>
                </Card.Header>
                <Card.Content>
                  <div className="list-stack">
                    {savedFilters.length ? (
                      savedFilters.map((preset) => (
                        <button
                          key={preset.preset_id}
                          type="button"
                          className="preset-row"
                          onClick={() => onApplyPreset?.(preset)}
                        >
                          <strong>{preset.name}</strong>
                          <span>{formatDateTime(preset.updated_at_ms)}</span>
                        </button>
                      ))
                    ) : (
                      <EmptyInline text="还没有保存的筛选器" />
                    )}
                  </div>
                </Card.Content>
              </Card>
              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>最近操作</Card.Title>
                </Card.Header>
                <Card.Content>
                  <div className="list-stack">
                    {recentOps.length ? (
                      recentOps.map((item) => (
                        <div key={`${item.time}-${item.text}`} className="audit-inline-row">
                          <strong>{item.text}</strong>
                          <span>{formatDateTime(item.time)}</span>
                        </div>
                      ))
                    ) : (
                      <EmptyInline text="当前没有最近操作记录" />
                    )}
                  </div>
                </Card.Content>
              </Card>
            </section>
          </>
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

  const needsText = ['reject', 'delete', 'comment', 'reply', 'blacklist', 'quick_reply', 'merge'].includes(action)
  const textPlaceholder =
    action === 'merge'
      ? '目标审核编号'
      : action === 'quick_reply'
        ? '快捷回复键名'
        : action === 'reject' || action === 'delete' || action === 'blacklist'
          ? '原因，可留空'
          : '内容'

  if (loading || !detail) {
    return <LoadingPanel text="正在加载详情" />
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

      {detail.render_png_blob_id && (
        <Card className="image-card" variant="secondary">
          <Card.Content>
            <img src={`/api/blobs/${detail.render_png_blob_id}`} alt="渲染预览" />
          </Card.Content>
        </Card>
      )}

      <Card className="detail-card" variant="secondary">
        <Card.Content>
          <dl className="kv">
            <div>
              <dt>分组</dt>
              <dd>{detail.group_id}</dd>
            </div>
            <div>
              <dt>投稿人</dt>
              <dd className="mono">{detail.sender_name || detail.sender_id || '-'}</dd>
            </div>
            <div>
              <dt>时间</dt>
              <dd>{formatDateTime(detail.created_at_ms)}</dd>
            </div>
            {detail.decision_reason && (
              <div>
                <dt>决策理由</dt>
                <dd>{detail.decision_reason}</dd>
              </div>
            )}
            <div>
              <dt>会话</dt>
              <dd className="mono">{detail.session_id}</dd>
            </div>
          </dl>
        </Card.Content>
      </Card>

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
              ariaLabel="详情动作"
              selectedKey={action}
              options={DETAIL_ACTIONS.map((item) => ({ value: item, label: ACTION_LABELS[item] ?? item }))}
              onSelect={setAction}
            />
            {action === 'defer' && (
              <DelayField value={actionDelay} onChange={onDelayChange} className="delay-field" />
            )}
            {needsText &&
              (action === 'reject' || action === 'delete' || action === 'comment' || action === 'reply' || action === 'blacklist' ? (
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

      <Card className="panel-card">
        <Card.Header>
          <Card.Title>时间线</Card.Title>
        </Card.Header>
        <Card.Content>
          <div className="timeline-list">
            {detail.timeline.map((item, index) => (
              <div key={`${item.label}-${index}`} className={`timeline-item timeline-${item.status}`}>
                <strong>{item.label}</strong>
                <span>{item.at_ms ? formatDateTime(item.at_ms) : '等待中'}</span>
                {item.detail ? <p>{item.detail}</p> : null}
              </div>
            ))}
          </div>
        </Card.Content>
      </Card>

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

function FailuresView({ notify }: { notify: (kind: ToastKind, text: string) => void }) {
  const [data, setData] = useState<FailureListResponse | null>(null)
  const [loading, setLoading] = useState(false)

  async function loadFailures() {
    setLoading(true)
    try {
      setData(await api<FailureListResponse>('/api/failures?limit=100'))
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void loadFailures()
  }, [])

  return (
    <div className="workspace">
      <header className="page-head">
        <div>
          <h1>失败中心</h1>
          <p>集中查看渲染失败、发布失败和稿件异常</p>
        </div>
        <Button size="sm" variant="secondary" onClick={loadFailures}>
          <RefreshCcw size={16} />
          刷新
        </Button>
      </header>

      {loading && !data ? (
        <LoadingPanel text="正在加载失败中心" />
      ) : !data ? (
        <EmptyPanel icon={<FileClock size={28} />} text="暂无失败数据" />
      ) : (
        <>
          <section className="metrics">
            <Metric label="异常总量" value={data.summary.total_count} tone="bad" icon={<AlertCircle size={18} />} />
            <Metric label="阶段失败" value={data.summary.stage_failed_count} tone="warn" icon={<Zap size={18} />} />
            <Metric label="渲染异常" value={data.summary.render_error_count} tone="bad" icon={<FileText size={18} />} />
            <Metric label="审核发布异常" value={data.summary.review_publish_error_count} tone="bad" icon={<Send size={18} />} />
          </section>
          <Card className="panel-card">
            <Card.Content>
              <div className="failure-table">
                {data.items.map((item) => (
                  <div key={`${item.post_id}-${item.source}`} className="failure-table-row">
                    <div>
                      <strong>#{item.review_code ?? '-'}</strong>
                      <span>{item.group_id} · {item.source}</span>
                    </div>
                    <p>{item.error}</p>
                    <small>{item.preview_text || '无文本预览'}</small>
                  </div>
                ))}
              </div>
            </Card.Content>
          </Card>
        </>
      )}
    </div>
  )
}

function BlacklistView({ notify }: { notify: (kind: ToastKind, text: string) => void }) {
  const [items, setItems] = useState<BlacklistItem[]>([])
  const [groupId, setGroupId] = useState('')
  const [senderId, setSenderId] = useState('')
  const [reason, setReason] = useState('')
  const [loading, setLoading] = useState(false)

  async function loadBlacklist() {
    setLoading(true)
    try {
      const result = await api<BlacklistListResponse>('/api/blacklist')
      setItems(result.items)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void loadBlacklist()
  }, [])

  async function addBlacklist() {
    if (!groupId.trim() || !senderId.trim()) {
      notify('info', '请填写分组和投稿人')
      return
    }
    try {
      await api('/api/blacklist', {
        method: 'POST',
        body: JSON.stringify({
          group_id: groupId.trim(),
          sender_id: senderId.trim(),
          reason: reason.trim() || null,
        }),
      })
      setSenderId('')
      setReason('')
      await loadBlacklist()
      notify('success', '已加入黑名单')
    } catch (error) {
      notify('error', (error as Error).message)
    }
  }

  async function removeBlacklist(item: BlacklistItem) {
    try {
      await api(`/api/blacklist/${encodeURIComponent(item.group_id)}/${encodeURIComponent(item.sender_id)}`, {
        method: 'POST',
      })
      await loadBlacklist()
      notify('success', '已移出黑名单')
    } catch (error) {
      notify('error', (error as Error).message)
    }
  }

  return (
    <div className="workspace">
      <header className="page-head">
        <div>
          <h1>黑名单管理</h1>
          <p>维护风险投稿人、原因和分组归属</p>
        </div>
        <Button size="sm" variant="secondary" onClick={loadBlacklist}>
          <RefreshCcw size={16} />
          刷新
        </Button>
      </header>

      <Card className="control-card">
        <Card.Content>
          <div className="toolbar-grid blacklist-form">
            <Input placeholder="分组 ID" value={groupId} onChange={(event) => setGroupId(event.target.value)} />
            <Input placeholder="投稿人 ID" value={senderId} onChange={(event) => setSenderId(event.target.value)} />
            <Input placeholder="拉黑原因" value={reason} onChange={(event) => setReason(event.target.value)} />
            <Button onClick={addBlacklist}>
              <Ban size={16} />
              加入黑名单
            </Button>
          </div>
        </Card.Content>
      </Card>

      <Card className="panel-card">
        <Card.Content>
          {loading ? (
            <LoadingPanel text="正在加载黑名单" />
          ) : items.length ? (
            <div className="list-stack">
              {items.map((item) => (
                <div key={`${item.group_id}-${item.sender_id}`} className="blacklist-row">
                  <div>
                    <strong>{item.sender_id}</strong>
                    <span>{item.group_id}</span>
                    {item.reason ? <p>{item.reason}</p> : null}
                  </div>
                  <Button size="sm" variant="danger-soft" onClick={() => void removeBlacklist(item)}>
                    移除
                  </Button>
                </div>
              ))}
            </div>
          ) : (
            <EmptyPanel icon={<Ban size={28} />} text="当前没有黑名单记录" />
          )}
        </Card.Content>
      </Card>
    </div>
  )
}

function AuditView({ notify }: { notify: (kind: ToastKind, text: string) => void }) {
  const [items, setItems] = useState<AuditListResponse['items']>([])
  const [loading, setLoading] = useState(false)

  async function loadAudit() {
    setLoading(true)
    try {
      const result = await api<AuditListResponse>('/api/audit?limit=100')
      setItems(result.items)
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void loadAudit()
  }, [])

  return (
    <div className="workspace">
      <header className="page-head">
        <div>
          <h1>操作审计</h1>
          <p>记录后台管理动作，便于追踪黑名单和筛选器等人工操作</p>
        </div>
        <Button size="sm" variant="secondary" onClick={loadAudit}>
          <RefreshCcw size={16} />
          刷新
        </Button>
      </header>
      <Card className="panel-card">
        <Card.Content>
          {loading ? (
            <LoadingPanel text="正在加载审计日志" />
          ) : items.length ? (
            <div className="list-stack">
              {items.map((item) => (
                <div key={item.audit_id} className="audit-row">
                  <div>
                    <strong>{item.summary}</strong>
                    <span>
                      {item.operator} · {item.action} · {item.group_id || '全局'}
                    </span>
                  </div>
                  <div className="audit-row-side">
                    <small>{formatDateTime(item.created_at_ms)}</small>
                    <Chip size="sm" variant="soft">
                      {item.status}
                    </Chip>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <EmptyPanel icon={<History size={28} />} text="当前还没有审计记录" />
          )}
        </Card.Content>
      </Card>
    </div>
  )
}

function StatsView({ notify }: { notify: (kind: ToastKind, text: string) => void }) {
  const [stats, setStats] = useState<StatsResponse | null>(null)
  const [loading, setLoading] = useState(false)

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

  useEffect(() => {
    void loadStats()
  }, [])

  if (loading && !stats) return <LoadingPanel text="正在加载统计" />
  if (!stats) return <EmptyPanel icon={<BarChart3 size={28} />} text="暂无统计数据" />

  return (
    <div className="workspace">
      <header className="page-head">
        <div>
          <h1>运行统计</h1>
          <p>保留原有统计能力，作为概览之外的详细数据页</p>
        </div>
        <Button size="sm" variant="secondary" onClick={loadStats}>
          <RefreshCcw size={16} />
          刷新
        </Button>
      </header>
      <section className="metrics">
        <Metric label="待审核" value={stats.pending_count} tone="warn" icon={<Clock3 size={18} />} />
        <Metric label="今日投稿" value={stats.today_count} tone="good" icon={<FileText size={18} />} />
        <Metric label="总投稿" value={stats.total_count} tone="neutral" icon={<Inbox size={18} />} />
        <Metric label="平均审核" value={formatDuration(stats.avg_review_time_ms)} tone="neutral" icon={<Users size={18} />} />
      </section>
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

function QuickEntryCard({
  title,
  text,
  icon,
  onClick,
}: {
  title: string
  text: string
  icon: React.ReactNode
  onClick: () => void
}) {
  return (
    <button type="button" className="quick-entry-card" onClick={onClick}>
      <div className="quick-entry-icon">{icon}</div>
      <strong>{title}</strong>
      <span>{text}</span>
    </button>
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
      : stage === 'failed' || stage === 'rejected' || stage === 'deleted' || stage === 'withdrawn'
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

function EmptyPanel({ icon, text }: { icon: React.ReactNode; text: string }) {
  return (
    <EmptyState className="empty-state">
      <div className="empty-icon">{icon}</div>
      <span>{text}</span>
    </EmptyState>
  )
}

function EmptyInline({ text }: { text: string }) {
  return <div className="empty-inline">{text}</div>
}

function LoadingPanel({ text }: { text: string }) {
  return (
    <div className="loading-panel">
      <Spinner size="sm" />
      <span>{text}</span>
    </div>
  )
}

function buildPostParams(filters: ReviewFilters) {
  const params = new URLSearchParams()
  if (filters.stage && filters.stage !== '__active__') params.set('stage', filters.stage)
  if (filters.stage === '__active__') params.set('active_only', 'true')
  if (filters.keyword.trim()) params.set('keyword', filters.keyword.trim())
  if (filters.groupId) params.set('group_id', filters.groupId)
  if (filters.onlyError) params.set('only_error', 'true')
  if (filters.onlyActionable) params.set('actionable_only', 'true')
  params.set('sort_by', filters.sortBy)
  params.set('sort_order', filters.sortOrder)
  params.set('cursor', String(filters.page * filters.pageSize))
  params.set('limit', String(filters.pageSize))
  return params
}

function buildActionPayload(action: string, text: string, delayMs = 180000) {
  const trimmed = text.trim()
  const payload: Record<string, unknown> = { action }
  if (action === 'defer') payload.delay_ms = delayMs
  if ((action === 'reject' || action === 'delete' || action === 'blacklist') && trimmed) payload.comment = trimmed
  if ((action === 'comment' || action === 'reply') && trimmed) payload.text = trimmed
  if (action === 'quick_reply' && trimmed) payload.quick_reply_key = trimmed
  if (action === 'merge' && trimmed) payload.target_review_code = Number(trimmed)
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

function showToast(kind: ToastKind, text: string) {
  if (kind === 'success') toast.success(text)
  else if (kind === 'error') toast.danger(text)
  else toast.info(text)
}

function FileImageIcon() {
  return <FileText size={20} />
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

export default App
