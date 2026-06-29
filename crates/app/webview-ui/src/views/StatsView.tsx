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
  STAGE_LABELS,
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
  type ExecuteGlobalActionBlock,
  type ExecuteReviewActionBlock,
  type InsertQueuedPostBlock,
  type NotificationMenuKey,
  type NotificationStageKey,
  type PostQuerySnapshot,
  type ReplyPrivateMessageBlock,
  type RuntimeConfigPageKey,
  type RuntimeConfigWorkbenchMode,
  type SelectOption,
  type SendWebhookBlock,
  type SettingsTabKey,
  type SortOrder,
  type TagMappingMenuKey,
  type ToastKind,
  buildActionPayload,
  buildPostParams,
  buildReplyImageLabel,
  cardActionLabel,
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

export function StatsView({ notify }: { notify: (kind: ToastKind, text: string) => void }) {
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
