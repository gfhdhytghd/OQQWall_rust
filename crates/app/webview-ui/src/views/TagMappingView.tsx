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
  SettingsMenuCard,
  StageChip,
  useMasonryLayout,
  useMediaQuery,
} from '../shared'

export function TagMappingView({ notify }: { notify: (kind: ToastKind, text: string) => void }) {
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
