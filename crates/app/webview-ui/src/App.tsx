import {
  type ComponentProps,
  type FormEvent,
  type Key,
  useEffect,
  useMemo,
  useState,
} from 'react'
import {
  Button,
  Card,
  Checkbox,
  Chip,
  EmptyState,
  Input,
  ListBox,
  Select,
  Spinner,
  Switch,
  TextArea,
  Toast,
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
  Filter,
  History,
  Inbox,
  LayoutDashboard,
  LogOut,
  RefreshCcw,
  Search,
  Send,
  ShieldCheck,
  Sparkles,
  Users,
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
const BATCH_ACTIONS = ['approve', 'reject', 'delete', 'skip', 'immediate', 'refresh', 'rerender']

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
        <EmptyPanel icon={<Spinner />} text="正在加载概览" />
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
  const [filters, setFilters] = useState<ReviewFilters>({
    stage: '__active__',
    keyword: '',
    groupId: '',
    sortBy: 'created_at',
    sortOrder: 'desc',
    onlyError: false,
    onlyActionable: false,
    page: 0,
    pageSize: 50,
  })
  const [savedFilters, setSavedFilters] = useState<SavedFilterPreset[]>([])
  const [selected, setSelected] = useState<string[]>([])
  const [batchAction, setBatchAction] = useState('approve')
  const [actionText, setActionText] = useState('')
  const [detail, setDetail] = useState<PostDetail | null>(null)
  const [senderHistory, setSenderHistory] = useState<PostItem[]>([])
  const [similarPosts, setSimilarPosts] = useState<SimilarPostResponse['items']>([])
  const [recentOps, setRecentOps] = useState<Array<{ time: number; text: string }>>([])
  const [presetName, setPresetName] = useState('')

  const groups = useMemo(() => [...new Set(posts.map((post) => post.group_id))].sort(), [posts])

  useEffect(() => {
    void loadPosts()
  }, [filters.stage, filters.groupId, filters.sortBy, filters.sortOrder, filters.onlyError, filters.onlyActionable, filters.page, filters.pageSize])

  useEffect(() => {
    void loadSavedFilters()
  }, [])

  async function loadPosts(resetSelection = false) {
    setLoading(true)
    try {
      const params = buildPostParams(filters)
      const result = await api<ListPostsResponse>('/api/posts?' + params.toString())
      setPosts(result.items)
      setTotal(result.total)
      if (resetSelection) setSelected([])
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
      // ignore sidebar helper data failure
    }
  }

  async function openDetail(post: PostItem) {
    try {
      const detailResult = await api<PostDetail>('/api/posts/' + post.post_id)
      setDetail(detailResult)
      if (detailResult.sender_id) {
        const history = await api<PostCollectionResponse>(
          '/api/posts/by-sender?' +
            new URLSearchParams({
              sender_id: detailResult.sender_id,
              group_id: detailResult.group_id,
              limit: '8',
            }).toString(),
        )
        setSenderHistory(history.items)
      } else {
        setSenderHistory([])
      }
      const similar = await api<SimilarPostResponse>(`/api/posts/${post.post_id}/similar?limit=6`)
      setSimilarPosts(similar.items)
    } catch (error) {
      notify('error', (error as Error).message)
    }
  }

  async function runAction(reviewId: string, action: string) {
    setActionLoading(true)
    try {
      await api(`/api/reviews/${reviewId}/decision`, {
        method: 'POST',
        body: JSON.stringify(buildActionPayload(action, actionText)),
      })
      pushRecentOp(`执行 ${ACTION_LABELS[action] ?? action}`)
      await loadPosts(true)
      if (detail?.post_id) {
        const current = posts.find((item) => item.post_id === detail.post_id)
        if (current) {
          await openDetail(current)
        }
      }
      notify('success', `已执行：${ACTION_LABELS[action] ?? action}`)
      setActionText('')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setActionLoading(false)
    }
  }

  async function runBatch() {
    if (!selected.length) return
    setActionLoading(true)
    try {
      await api('/api/reviews/batch', {
        method: 'POST',
        body: JSON.stringify({
          review_ids: selected,
          ...buildActionPayload(batchAction, actionText),
        }),
      })
      pushRecentOp(`批量执行 ${ACTION_LABELS[batchAction] ?? batchAction}（${selected.length} 条）`)
      setSelected([])
      setActionText('')
      await loadPosts(true)
      notify('success', '批量执行完成')
    } catch (error) {
      notify('error', (error as Error).message)
    } finally {
      setActionLoading(false)
    }
  }

  async function selectAcrossPages() {
    try {
      const params = buildPostParams({ ...filters, page: 0 })
      params.delete('cursor')
      params.delete('limit')
      const result = await api<ListReviewIdsResponse>('/api/reviews/ids?' + params.toString())
      setSelected(result.review_ids)
      notify(result.total ? 'success' : 'info', result.total ? `已选择 ${result.total} 条` : '没有可选择稿件')
    } catch (error) {
      notify('error', (error as Error).message)
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
            stage: filters.stage,
            keyword: filters.keyword,
            group_id: filters.groupId,
            sort_by: filters.sortBy,
            sort_order: filters.sortOrder,
            only_error: filters.onlyError,
            only_actionable: filters.onlyActionable,
            page_size: filters.pageSize,
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
    setFilters((prev) => ({
      ...prev,
      stage: (preset.query.stage as Stage) || '__active__',
      keyword: preset.query.keyword || '',
      groupId: preset.query.group_id || '',
      sortBy: preset.query.sort_by || 'created_at',
      sortOrder: (preset.query.sort_order as SortOrder) || 'desc',
      onlyError: preset.query.only_error,
      onlyActionable: preset.query.only_actionable,
      pageSize: preset.query.page_size || prev.pageSize,
      page: 0,
    }))
  }

  return (
    <div className="workspace">
      <header className="page-head">
        <div>
          <h1>稿件主操作台</h1>
          <p>把筛选、批量审核、详情、时间线和关联稿件整合到一个工作区</p>
        </div>
        <div className="head-actions">
          <Switch isSelected={filters.onlyError} onChange={(value) => setFilters((prev) => ({ ...prev, onlyError: value }))} size="sm">
            仅异常
          </Switch>
          <Button size="sm" variant="secondary" onClick={() => loadPosts()}>
            <RefreshCcw size={16} />
            刷新
          </Button>
        </div>
      </header>

      <section className="metrics">
        <Metric label="当前结果" value={posts.length} tone="neutral" icon={<Inbox size={18} />} />
        <Metric label="已选稿件" value={selected.length} tone="warn" icon={<Check size={18} />} />
        <Metric label="可操作" value={posts.filter((item) => !!item.review_id).length} tone="good" icon={<CheckCircle2 size={18} />} />
        <Metric label="异常稿件" value={posts.filter((item) => !!item.last_error).length} tone="bad" icon={<AlertCircle size={18} />} />
      </section>

      <div className="review-layout">
        <section className="review-main-column">
          <Card className="control-card">
            <Card.Content>
              <div className="toolbar-grid">
                <HeroSelect
                  ariaLabel="状态筛选"
                  selectedKey={filters.stage}
                  options={STAGE_OPTIONS}
                  onSelect={(value) => setFilters((prev) => ({ ...prev, stage: value as Stage, page: 0 }))}
                />
                <HeroSelect
                  ariaLabel="分组筛选"
                  selectedKey={filters.groupId}
                  options={[{ value: '', label: '全部分组' }, ...groups.map((group) => ({ value: group, label: group }))]}
                  onSelect={(value) => setFilters((prev) => ({ ...prev, groupId: value, page: 0 }))}
                />
                <HeroSelect
                  ariaLabel="排序"
                  selectedKey={`${filters.sortBy}:${filters.sortOrder}`}
                  options={SORT_OPTIONS}
                  onSelect={(value) => {
                    const [sortBy, sortOrder] = value.split(':')
                    setFilters((prev) => ({ ...prev, sortBy, sortOrder: sortOrder as SortOrder, page: 0 }))
                  }}
                />
                <Input
                  className="search-control"
                  placeholder="搜索编号、投稿人、内容、错误"
                  value={filters.keyword}
                  onChange={(event) => setFilters((prev) => ({ ...prev, keyword: event.target.value }))}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') void loadPosts(true)
                  }}
                />
                <Button variant="secondary" onClick={() => loadPosts(true)}>
                  <Search size={16} />
                  搜索
                </Button>
                <Button
                  variant="tertiary"
                  onClick={() =>
                    setFilters({
                      stage: '__active__',
                      keyword: '',
                      groupId: '',
                      sortBy: 'created_at',
                      sortOrder: 'desc',
                      onlyError: false,
                      onlyActionable: false,
                      page: 0,
                      pageSize: 50,
                    })
                  }
                >
                  重置
                </Button>
              </div>
              <div className="filter-row">
                <Checkbox isSelected={filters.onlyActionable} onChange={(value) => setFilters((prev) => ({ ...prev, onlyActionable: value }))}>
                  可操作
                </Checkbox>
                <Checkbox isSelected={filters.onlyError} onChange={(value) => setFilters((prev) => ({ ...prev, onlyError: value }))}>
                  仅异常
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
                  <Button size="sm" variant="secondary" onClick={selectAcrossPages}>
                    选择当前筛选
                  </Button>
                  <Button size="sm" variant="secondary" onClick={() => setSelected([])}>
                    清空选择
                  </Button>
                </div>
                <div className="batch-actions batch-actions-right">
                  <HeroSelect
                    ariaLabel="批量动作"
                    selectedKey={batchAction}
                    options={BATCH_ACTIONS.map((action) => ({ value: action, label: ACTION_LABELS[action] ?? action }))}
                    onSelect={setBatchAction}
                  />
                  <Input
                    className="action-text"
                    placeholder="评论、拒绝或删除原因"
                    value={actionText}
                    onChange={(event) => setActionText(event.target.value)}
                  />
                  <Button size="sm" isDisabled={!selected.length || actionLoading} onClick={runBatch}>
                    {actionLoading ? <Spinner size="sm" /> : <Send size={16} />}
                    批量执行
                  </Button>
                </div>
              </div>
            </Card.Content>
          </Card>

          <Card className="panel-card">
            <Card.Header>
              <Card.Title>稿件列表</Card.Title>
            </Card.Header>
            <Card.Content>
              {loading ? (
                <EmptyPanel icon={<Spinner />} text="正在加载稿件" />
              ) : posts.length ? (
                <div className="post-table-simple">
                  {posts.map((post) => (
                    <button key={post.post_id} className="post-line" type="button" onClick={() => void openDetail(post)}>
                      <span className="post-line-check">
                        {post.review_id ? (
                          <Checkbox
                            aria-label={`选择 ${post.post_id}`}
                            isSelected={selected.includes(post.review_id)}
                            onChange={(checked) =>
                              setSelected((prev) =>
                                checked
                                  ? [...new Set([...prev, post.review_id!])]
                                  : prev.filter((item) => item !== post.review_id),
                              )
                            }
                          />
                        ) : null}
                      </span>
                      <span className="post-line-code">#{post.internal_code ?? post.external_code ?? '-'}</span>
                      <span className="post-line-stage">
                        <StageChip stage={post.stage} />
                      </span>
                      <span className="post-line-main">
                        <strong>{post.preview_text || '点击查看稿件详情'}</strong>
                        <small>
                          {post.group_id} · {post.sender_id ?? '未知投稿人'} · {formatDateTime(post.created_at_ms)}
                        </small>
                      </span>
                    </button>
                  ))}
                </div>
              ) : (
                <EmptyPanel icon={<FileSearch size={28} />} text="没有符合条件的稿件" />
              )}
              <div className="pager-row">
                <span>共 {total} 条</span>
                <HeroSelect
                  ariaLabel="分页大小"
                  selectedKey={String(filters.pageSize)}
                  options={PAGE_SIZES.map((size) => ({ value: String(size), label: `${size} 条/页` }))}
                  onSelect={(value) => setFilters((prev) => ({ ...prev, pageSize: Number(value), page: 0 }))}
                />
              </div>
            </Card.Content>
          </Card>
        </section>

        <aside className="review-side-column">
          <Card className="panel-card">
            <Card.Header>
              <Card.Title>已保存筛选</Card.Title>
            </Card.Header>
            <Card.Content>
              <div className="list-stack">
                {savedFilters.length ? (
                  savedFilters.map((preset) => (
                    <button key={preset.preset_id} type="button" className="preset-row" onClick={() => applyPreset(preset)}>
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

          <Card className="panel-card detail-panel">
            <Card.Header>
              <Card.Title>详情侧栏</Card.Title>
            </Card.Header>
            <Card.Content>
              {detail ? (
                <DetailSidebar
                  detail={detail}
                  senderHistory={senderHistory}
                  similarPosts={similarPosts}
                  actionLoading={actionLoading}
                  actionText={actionText}
                  onActionTextChange={setActionText}
                  onAction={(action) => detail.review_id && runAction(detail.review_id, action)}
                />
              ) : (
                <EmptyInline text="从左侧稿件列表打开一条稿件" />
              )}
            </Card.Content>
          </Card>
        </aside>
      </div>
    </div>
  )
}

function DetailSidebar({
  detail,
  senderHistory,
  similarPosts,
  actionLoading,
  actionText,
  onActionTextChange,
  onAction,
}: {
  detail: PostDetail
  senderHistory: PostItem[]
  similarPosts: SimilarPostResponse['items']
  actionLoading: boolean
  actionText: string
  onActionTextChange: (value: string) => void
  onAction: (action: string) => void
}) {
  return (
    <div className="detail-sidebar-body">
      <div className="detail-meta-grid">
        <StageChip stage={detail.stage} />
        <Chip size="sm" variant="soft">
          {detail.group_id}
        </Chip>
        <Chip size="sm" variant="soft">
          {detail.sender_name || detail.sender_id || '未知投稿人'}
        </Chip>
      </div>

      <div className="detail-actions">
        {['approve', 'reject', 'delete', 'immediate', 'rerender', 'blacklist'].map((action) => (
          <Button
            key={action}
            size="sm"
            variant={action === 'approve' ? 'primary' : action === 'reject' || action === 'delete' || action === 'blacklist' ? 'danger-soft' : 'secondary'}
            isDisabled={!detail.review_id || actionLoading}
            onClick={() => onAction(action)}
          >
            {ACTION_LABELS[action] ?? action}
          </Button>
        ))}
      </div>

      <TextArea
        className="action-text"
        placeholder="评论、拒绝/删除原因、拉黑原因"
        value={actionText}
        onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) => onActionTextChange(event.target.value)}
      />

      <Card className="detail-card" variant="secondary">
        <Card.Content>
          <dl className="kv">
            <div>
              <dt>审核编号</dt>
              <dd>#{detail.review_code ?? detail.external_code ?? '-'}</dd>
            </div>
            <div>
              <dt>创建时间</dt>
              <dd>{formatDateTime(detail.created_at_ms)}</dd>
            </div>
            <div>
              <dt>会话</dt>
              <dd>{detail.session_id}</dd>
            </div>
          </dl>
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

      <Card className="panel-card">
        <Card.Header>
          <Card.Title>投稿人历史稿件</Card.Title>
        </Card.Header>
        <Card.Content>
          <div className="list-stack">
            {senderHistory.length ? (
              senderHistory.map((item) => (
                <div key={item.post_id} className="history-row">
                  <strong>#{item.internal_code ?? item.external_code ?? '-'}</strong>
                  <span>{item.preview_text || '无文本预览'}</span>
                </div>
              ))
            ) : (
              <EmptyInline text="暂无历史稿件" />
            )}
          </div>
        </Card.Content>
      </Card>

      <Card className="panel-card">
        <Card.Header>
          <Card.Title>相似稿件</Card.Title>
        </Card.Header>
        <Card.Content>
          <div className="list-stack">
            {similarPosts.length ? (
              similarPosts.map((item) => (
                <div key={item.post.post_id} className="history-row">
                  <strong>{item.similarity_reason}</strong>
                  <span>{item.post.preview_text || '无文本预览'}</span>
                </div>
              ))
            ) : (
              <EmptyInline text="暂无相似稿件" />
            )}
          </div>
        </Card.Content>
      </Card>
    </div>
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
        <EmptyPanel icon={<Spinner />} text="正在加载失败中心" />
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
            <EmptyPanel icon={<Spinner />} text="正在加载黑名单" />
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
            <EmptyPanel icon={<Spinner />} text="正在加载审计日志" />
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
                  <small>{formatDateTime(item.created_at_ms)}</small>
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

  if (loading && !stats) return <EmptyPanel icon={<Spinner />} text="正在加载统计" />
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

function buildActionPayload(action: string, text: string) {
  const trimmed = text.trim()
  const payload: Record<string, unknown> = { action }
  if ((action === 'reject' || action === 'delete' || action === 'blacklist') && trimmed) payload.comment = trimmed
  if ((action === 'comment' || action === 'reply') && trimmed) payload.text = trimmed
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

export default App
