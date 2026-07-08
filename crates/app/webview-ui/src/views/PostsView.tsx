import {
  type CSSProperties,
  type DragEvent,
  type ReactNode,
  useEffect,
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
  Input,
  Pagination,
  Popover,
  Spinner,
  Switch,
  TextArea,
  ToggleButton,
  ToggleButtonGroup,
} from '@heroui/react'
import {
  AlertCircle,
  BarChart3,
  Ban,
  Check,
  CheckCircle2,
  Clock3,
  FileText,
  HelpCircle,
  Inbox,
  LayoutGrid,
  List,
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
import { api } from '../api/client'
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
  Stage,
  StatsResponse,
  ConfigAdminEntry,
  TagValueMappingEntry,
  TagValueMappingGroup,
  UserNotificationSettingsResponse,
  UserNotificationTemplate,
} from '../api/types'
import {
  ACTIVE_EXCLUDED,
  AGENT_BLOCK_DRAG_MIME,
  AGENT_COMMAND_BLOCK_OPTIONS,
  AGENT_GLOBAL_ACTION_OPTIONS,
  AGENT_QUEUE_INSERT_POSITION_OPTIONS,
  AGENT_REVIEW_ACTION_OPTIONS,
  AGENT_SHORTCUT_SCOPE_OPTIONS,
  BATCH_ACTIONS,
  CARD_QUICK_ACTIONS,
  DANGEROUS_ACTIONS,
  DETAIL_ACTIONS,
  DETAIL_QUICK_ACTIONS,
  LIST_PRIMARY_ACTIONS,
  NOTIFICATION_STAGE_OPTIONS,
  PAGE_SIZES,
  SORT_OPTIONS,
  type AgentBlockDragPayload,
  type AgentCommandBlockOptionValue,
  type AgentCommandFieldNode,
  type AgentGlobalActionKey,
  type AgentReviewActionKey,
  type AgentVariableDragPayload,
  type ConfigMenuKey,
  type ExecuteGlobalActionBlock,
  type ExecuteReviewActionBlock,
  type InsertQueuedPostBlock,
  type NotificationMenuKey,
  type NotificationStageKey,
  type PostQuerySnapshot,
  type PostViewMode,
  type ReplyPrivateMessageBlock,
  type RuntimeConfigPageKey,
  type RuntimeConfigWorkbenchMode,
  type SelectOption,
  type SendWebhookBlock,
  type SettingsTabKey,
  type SortOrder,
  type TagMappingMenuKey,
  type ToastKind,
  type PostsWorkspaceMode,
  buildActionPayload,
  buildPostParams,
  buildReplyImageLabel,
  cardActionLabel,
  DelayField,
  EmptyPanel,
  formatDateTime,
  formatDuration,
  HeroSelect,
  isPreviewableImageSource,
  Metric,
  quickActionIcon,
  quickActionVariant,
  readFileAsDataUrl,
  StageChip,
  useMasonryLayout,
  useMediaQuery,
} from '../shared'

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

export function PostsView({
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
          <h1>{title}</h1>
          <p>{description}</p>
          <span className="page-meta">
            {lastUpdatedAt ? `刷新 ${formatDateTime(lastUpdatedAt)}` : '等待刷新'}
          </span>
        </div>
        <div className="head-actions">
          <div className="layout-note">
            <PanelRightOpen size={16} />
            {mode === 'review' ? '双栏审核' : '记录浏览'}
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
              {allowOnlyError ? <Checkbox isSelected={onlyError} onChange={setOnlyError}>仅异常</Checkbox> : null}
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
                    ? '取消本页选择'
                    : '选择本页'}
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
                <h2>{mode === 'sent' ? '发送记录' : '稿件队列'}</h2>
                <p>{description}</p>
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
                <EmptyPanel icon={<Inbox size={28} />} text={emptyText} />
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
