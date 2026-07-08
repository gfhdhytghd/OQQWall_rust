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
  ArrowLeft,
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
  X,
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
  MappingListEditor,
  Metric,
  quickActionIcon,
  quickActionVariant,
  readFileAsDataUrl,
  SettingsMenuCard,
  StageChip,
  useMasonryLayout,
  useMediaQuery,
} from '../shared'
import { AgentCommandWorkbench } from './AgentCommandWorkbench'
import { NotificationSettingsWorkbench } from './NotificationSettingsView'

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

export function SettingsView({
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
      label: '运行配置',
      description: 'config.json 和运行时参数',
      icon: <LayoutGrid size={16} />,
    },
    {
      value: 'notifications' as const,
      label: '用户回链',
      description: '入队、发稿成功和拒稿消息',
      icon: <Send size={16} />,
    },
  ]

  return (
    <div className="workspace settings-hub">
      <Card className="panel-card settings-tabs-card">
        <Card.Content>
          <div className="settings-tabs-head">
            <h2>设置</h2>
          </div>
          <div className="settings-tab-list" role="tablist" aria-label="设置分类">
            {options.map((option) => (
              <button
                key={option.value}
                type="button"
                role="tab"
                aria-selected={tab === option.value}
                className={`settings-tab-button ${tab === option.value ? 'is-active' : ''}`}
                onClick={() => setTab(option.value)}
              >
                <span className="settings-stage-icon">{option.icon}</span>
                <span className="settings-tab-text">
                  <strong>{option.label}</strong>
                  <span>{option.description}</span>
                </span>
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
}

export function AgentView({
  notify,
}: {
  notify: (kind: ToastKind, text: string) => void
}) {
  return <RuntimeConfigWorkbench notify={notify} mode="agent" />
}

type OperationGroupPageKey =
  | 'operation_basic'
  | 'operation_inbox'
  | 'operation_delivery'
  | 'operation_agent'
  | 'operation_misc'

function buildRuntimeConfigPages(): Array<{
  value: RuntimeConfigPageKey
  label: string
  description: string
  icon: ReactNode
}> {
  return [
    {
      value: 'runtime_settings',
      label: '运行设置',
      description: '服务端口、默认运行参数和遥测。',
      icon: <Clock3 size={16} />,
    },
    {
      value: 'operation_settings',
      label: '运营设置',
      description: '进入组卡片，再维护具体组配置。',
      icon: <Inbox size={16} />,
    },
    {
      value: 'operation_global_misc',
      label: '全局杂项',
      description: '全局 WebUI 管理员。',
      icon: <ShieldCheck size={16} />,
    },
  ]
}

function buildOperationGroupPages(): Array<{
  value: OperationGroupPageKey
  label: string
  description: string
  icon: ReactNode
}> {
  return [
    {
      value: 'operation_basic',
      label: '基础配置',
      description: '组标识、审核群和 NapCat 接入。',
      icon: <Inbox size={16} />,
    },
    {
      value: 'operation_inbox',
      label: '收件设置',
      description: '收件等待、好友申请和自动私信。',
      icon: <MessageSquare size={16} />,
    },
    {
      value: 'operation_delivery',
      label: '发件设置',
      description: '账号池、定时发稿、队列和图片限制。',
      icon: <Send size={16} />,
    },
    {
      value: 'operation_agent',
      label: 'Agent',
      description: 'Agent 指令和触发管理员。',
      icon: <FileText size={16} />,
    },
    {
      value: 'operation_misc',
      label: '杂项',
      description: '快捷回复、快捷指令、水印和组管理员。',
      icon: <MoreHorizontal size={16} />,
    },
  ]
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

  function updateAgentCommandAdmin(index: number, value: string) {
    updateSelectedGroup((group) => ({
      ...group,
      agent_command_admins: group.agent_command_admins.map((item, itemIndex) =>
        itemIndex === index ? value : item
      ),
    }))
  }

  function addAgentCommandAdmin() {
    updateSelectedGroup((group) => ({
      ...group,
      agent_command_admins: [...group.agent_command_admins, ''],
    }))
  }

  function removeAgentCommandAdmin(index: number) {
    updateSelectedGroup((group) => ({
      ...group,
      agent_command_admins: group.agent_command_admins.filter((_, itemIndex) => itemIndex !== index),
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
  const [selectedMenu, setSelectedMenu] = useState<ConfigMenuKey>('runtime_settings')
  const [selectedOperationPage, setSelectedOperationPage] =
    useState<OperationGroupPageKey>('operation_basic')
  const [operationGroupDetailOpen, setOperationGroupDetailOpen] = useState(false)
  const [mobileConfigDetailOpen, setMobileConfigDetailOpen] = useState(false)
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const compactRuntimeConfig = useMediaQuery('(max-width: 640px)')
  const runtimeConfigPages = buildRuntimeConfigPages()
  const operationGroupPages = buildOperationGroupPages()
  const selectedRuntimeConfigPage =
    runtimeConfigPages.find((page) => page.value === selectedMenu) ?? runtimeConfigPages[0]

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

  function scrollRuntimeConfigTop() {
    window.requestAnimationFrame(() => {
      window.scrollTo({ top: 0, left: 0, behavior: 'auto' })
    })
  }

  function selectRuntimeConfigPage(value: ConfigMenuKey) {
    setSelectedMenu(value)
    if (value === 'operation_settings') {
      setOperationGroupDetailOpen(false)
    }
    if (compactRuntimeConfig && mode === 'full') {
      setMobileConfigDetailOpen(true)
      scrollRuntimeConfigTop()
    }
  }

  function openOperationGroup(groupId: string) {
    setSelectedGroup(groupId)
    setSelectedOperationPage('operation_basic')
    setOperationGroupDetailOpen(true)
    scrollRuntimeConfigTop()
  }

  function returnToOperationGroups() {
    setOperationGroupDetailOpen(false)
    scrollRuntimeConfigTop()
  }

  function returnToRuntimeConfigMenu() {
    setMobileConfigDetailOpen(false)
    scrollRuntimeConfigTop()
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

  function updateAgentCommandAdmin(index: number, value: string) {
    updateSelectedGroup((group) => ({
      ...group,
      agent_command_admins: group.agent_command_admins.map((item, itemIndex) =>
        itemIndex === index ? value : item
      ),
    }))
  }

  function addAgentCommandAdmin() {
    updateSelectedGroup((group) => ({
      ...group,
      agent_command_admins: [...group.agent_command_admins, ''],
    }))
  }

  function removeAgentCommandAdmin(index: number) {
    updateSelectedGroup((group) => ({
      ...group,
      agent_command_admins: group.agent_command_admins.filter((_, itemIndex) => itemIndex !== index),
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
    if (nextGroupId) {
      setSelectedGroup(nextGroupId)
      if (mode === 'full' && selectedMenu === 'operation_settings') {
        setSelectedOperationPage('operation_basic')
        setOperationGroupDetailOpen(true)
      }
    }
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
    setOperationGroupDetailOpen(false)
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
  const groupSpecificRuntimeSummary = describeGroupSpecificRuntimeSettings(groups)
  if (mode === 'agent') {
    return (
      <div className="workspace settings-workspace agent-workspace">
        <header className="page-head">
          <div>
            <h1>Scratch Agent</h1>
            <p>用 Scratch 风格工作区编排用户指令流。</p>
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
                <Card.Title>Scratch 工作台</Card.Title>
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

  function renderOperationGroupCards() {
    return (
      <Card className="panel-card operation-groups-card">
        <Card.Header>
          <Card.Title>组</Card.Title>
          <div className="head-actions">
            <Button size="sm" variant="secondary" onClick={addGroup}>
              新增分组
            </Button>
          </div>
        </Card.Header>
        <Card.Content>
          {groups.length > 0 ? (
            <div className="operation-group-grid">
              {groups.map((group, index) => (
                <button
                  key={`${group.group_id || 'group'}-${index}`}
                  type="button"
                  className={`settings-stage-button operation-group-card ${
                    selectedGroup === group.group_id ? 'is-active' : ''
                  }`}
                  onClick={() => openOperationGroup(group.group_id)}
                >
                  <span className="settings-stage-icon">
                    <Inbox size={16} />
                  </span>
                  <strong>{group.group_id || '未命名分组'}</strong>
                  <span>审核群：{group.audit_group_id || '未填写'}</span>
                  <span>发件账号：{group.accounts.length} 个</span>
                  <span>定时发稿：{group.send_schedule.length} 个时间点</span>
                </button>
              ))}
            </div>
          ) : (
            <EmptyPanel icon={<LayoutGrid size={28} />} text="当前没有可用的分组配置" />
          )}
        </Card.Content>
      </Card>
    )
  }

  function renderOperationGroupDetailHead() {
    if (!activeGroup) return null
    return (
      <Card className="panel-card operation-group-detail-card">
        <Card.Content>
          <div className="operation-group-detail-head">
            <Button size="sm" variant="secondary" onClick={returnToOperationGroups}>
              <ArrowLeft size={16} />
              返回组列表
            </Button>
            <div className="operation-group-detail-title">
              <span className="field-label">当前组</span>
              <strong>{activeGroup.group_id || '未命名分组'}</strong>
              <span>审核群：{activeGroup.audit_group_id || '未填写'}</span>
            </div>
            <div className="head-actions">
              <Button size="sm" variant="secondary" onClick={addGroup}>
                新增分组
              </Button>
              <Button size="sm" variant="secondary" onClick={removeCurrentGroup}>
                删除当前组
              </Button>
            </div>
          </div>
        </Card.Content>
      </Card>
    )
  }

  function renderOperationGroupPageCards() {
    return (
      <Card className="panel-card operation-group-pages-card">
        <Card.Content>
          <div className="settings-stage-grid operation-setting-grid">
            {operationGroupPages.map((page) => (
              <button
                key={page.value}
                type="button"
                className={`settings-stage-button ${
                  selectedOperationPage === page.value ? 'is-active' : ''
                }`}
                onClick={() => setSelectedOperationPage(page.value)}
              >
                <span className="settings-stage-icon">{page.icon}</span>
                <strong>{page.label}</strong>
                <span>{page.description}</span>
              </button>
            ))}
          </div>
        </Card.Content>
      </Card>
    )
  }

  function renderRuntimeConfigActions() {
    return (
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
    )
  }

  const showMobileConfigDetail = compactRuntimeConfig && mobileConfigDetailOpen
  const showRuntimeConfigMenu = !showMobileConfigDetail
  const showRuntimeConfigContent = !compactRuntimeConfig || mobileConfigDetailOpen
  const showOperationGroupDetail =
    selectedMenu === 'operation_settings' && operationGroupDetailOpen && Boolean(activeGroup)
  const showOperationGroupList = selectedMenu === 'operation_settings' && !showOperationGroupDetail

  return (
    <div
      className={`workspace settings-workspace ${
        showMobileConfigDetail ? 'is-mobile-config-detail' : ''
      }`}
    >
      {!showMobileConfigDetail ? (
        <header className="page-head">
          <div>
            <h1>设置</h1>
            <p>按运行设置和运营设置分组维护 config.json。</p>
          </div>
          {renderRuntimeConfigActions()}
        </header>
      ) : null}

      <section className="settings-layout runtime-settings-layout">
        {showRuntimeConfigMenu ? (
          <SettingsMenuCard
            title="运行配置页面"
            options={runtimeConfigPages}
            activeKey={selectedMenu}
            onSelect={selectRuntimeConfigPage}
          />
        ) : null}

        {showRuntimeConfigContent ? (
          <div className="settings-content-stack">
            {showMobileConfigDetail ? (
              <div className="settings-mobile-detail-head">
                <div className="settings-mobile-detail-top">
                  <Button size="sm" variant="secondary" onClick={returnToRuntimeConfigMenu}>
                    <ArrowLeft size={16} />
                    返回
                  </Button>
                  {renderRuntimeConfigActions()}
                </div>
                <div>
                  <strong>{selectedRuntimeConfigPage.label}</strong>
                  <span>{selectedRuntimeConfigPage.description}</span>
                </div>
              </div>
            ) : null}

          {selectedMenu === 'runtime_settings' ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>运行设置</Card.Title>
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
                        placeholder={groupSpecificRuntimeSummary ? '留空表示只使用分组配置' : undefined}
                        value={settings.common.napcat_base_url}
                        onChange={(event) => updateCommon('napcat_base_url', event.target.value)}
                      />
                    </div>
                    <div className="field-stack config-span-2">
                      <span className="field-label">NapCat Access Token</span>
                      <Input
                        placeholder={groupSpecificRuntimeSummary ? '留空表示只使用分组配置' : undefined}
                        value={settings.common.napcat_access_token}
                        onChange={(event) =>
                          updateCommon('napcat_access_token', event.target.value)
                        }
                      />
                    </div>
                    <div className="field-stack config-span-2">
                      <span className="field-label">好友通过自动私信</span>
                      <TextArea
                        placeholder={groupSpecificRuntimeSummary ? '留空表示只使用分组配置' : undefined}
                        value={settings.common.friend_add_message}
                        onChange={(event: React.ChangeEvent<HTMLTextAreaElement>) =>
                          updateCommon('friend_add_message', event.target.value)
                        }
                      />
                    </div>
                    {groupSpecificRuntimeSummary ? (
                      <div className="field-hint config-span-2">
                        公共 NapCat 和好友私信留空不会清空分组配置；{groupSpecificRuntimeSummary}
                      </div>
                    ) : null}
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

          {selectedMenu === 'runtime_settings' ? (
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

          {selectedMenu === 'runtime_settings' ? (
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

          {selectedMenu === 'operation_global_misc' ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>全局杂项</Card.Title>
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

          {showOperationGroupList ? renderOperationGroupCards() : null}

          {showOperationGroupDetail ? (
            <>
              {renderOperationGroupDetailHead()}
              {renderOperationGroupPageCards()}
            </>
          ) : null}

          {showOperationGroupDetail && selectedOperationPage === 'operation_basic' && activeGroup ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>基础配置</Card.Title>
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
                </div>
              </Card.Content>
            </Card>
          ) : null}

          {showOperationGroupDetail && selectedOperationPage === 'operation_inbox' && activeGroup ? (
            <Card className="panel-card">
              <Card.Header>
                <Card.Title>收件设置</Card.Title>
              </Card.Header>
              <Card.Content>
                <div className="config-form-grid">
                  <ConfigNumberField
                    label="处理等待（秒）"
                    value={activeGroup.process_waittime_sec}
                    min={0}
                    onChange={(value) => updateGroupField('process_waittime_sec', value)}
                  />
                  <ConfigNumberField
                    label="好友请求窗口（秒）"
                    value={activeGroup.friend_request_window_sec}
                    min={0}
                    onChange={(value) => updateGroupField('friend_request_window_sec', value)}
                  />
                  <div className="field-stack config-switch-row">
                    <span className="field-label">启用指令式收稿</span>
                    <Switch
                      isSelected={activeGroup.submission_session_enabled}
                      size="sm"
                      onChange={(value) =>
                        updateSelectedGroup((group) => ({
                          ...group,
                          submission_session_enabled: value,
                          submission_session_required: value
                            ? group.submission_session_required
                            : false,
                        }))
                      }
                    >
                      {activeGroup.submission_session_enabled ? '已启用' : '已关闭'}
                    </Switch>
                  </div>
                  <div className="field-stack config-switch-row">
                    <span className="field-label">仅指令式收稿</span>
                    <Switch
                      isSelected={activeGroup.submission_session_required}
                      isDisabled={!activeGroup.submission_session_enabled}
                      size="sm"
                      onChange={(value) =>
                        updateSelectedGroup((group) => ({
                          ...group,
                          submission_session_enabled: value ? true : group.submission_session_enabled,
                          submission_session_required: value,
                        }))
                      }
                    >
                      {activeGroup.submission_session_required ? '已启用' : '已关闭'}
                    </Switch>
                  </div>
                  <div className="field-stack config-switch-row">
                    <span className="field-label">合并文本到首条消息</span>
                    <Switch
                      isSelected={activeGroup.submission_session_merge_text_to_first_message}
                      isDisabled={!activeGroup.submission_session_enabled}
                      size="sm"
                      onChange={(value) =>
                        updateGroupField('submission_session_merge_text_to_first_message', value)
                      }
                    >
                      {activeGroup.submission_session_merge_text_to_first_message
                        ? '已启用'
                        : '已关闭'}
                    </Switch>
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

          {showOperationGroupDetail && selectedOperationPage === 'operation_delivery' && activeGroup ? (
            <div className="config-card-grid">
              <div className="config-subcard">
                <div className="settings-panel">
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
                  <div className="field-stack">
                    <span className="field-label">渲染水印文本</span>
                    <Input
                      value={activeGroup.watermark_text}
                      onChange={(event) => updateGroupField('watermark_text', event.target.value)}
                    />
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

          {showOperationGroupDetail && selectedOperationPage === 'operation_agent' && activeGroup ? (
            <>
              <Card className="panel-card agent-workbench-card">
                <Card.Header>
                  <Card.Title>Agent</Card.Title>
                </Card.Header>
                <Card.Content>
                  <AgentCommandWorkbench
                    commands={activeGroup.agent_commands}
                    variables={settings.agent_command_variables}
                    onChange={(commands) => updateGroupField('agent_commands', commands)}
                  />
                </Card.Content>
              </Card>
              <Card className="panel-card">
                <Card.Header>
                  <Card.Title>Agent 管理员</Card.Title>
                </Card.Header>
                <Card.Content>
                  <StringListEditor
                    label="Agent 指令管理员 QQ"
                    hint="开启“仅管理员”的 Agent 指令只允许这些 QQ 号触发。"
                    addLabel="新增 QQ"
                    placeholder="QQ 号"
                    values={activeGroup.agent_command_admins}
                    onAdd={addAgentCommandAdmin}
                    onChange={updateAgentCommandAdmin}
                    onRemove={removeAgentCommandAdmin}
                  />
                </Card.Content>
              </Card>
            </>
          ) : null}

          {showOperationGroupDetail && selectedOperationPage === 'operation_misc' && activeGroup ? (
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

          {showOperationGroupDetail && selectedOperationPage === 'operation_misc' && activeGroup ? (
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

          {showOperationGroupDetail && selectedOperationPage === 'operation_misc' && activeGroup ? (
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
        ) : null}
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

function describeGroupSpecificRuntimeSettings(groups: AppConfigGroupSettings[]) {
  const configuredGroups = groups
    .filter(
      (group) =>
        group.napcat_base_url.trim() ||
        group.napcat_access_token.trim() ||
        group.friend_add_message.trim()
    )
    .map((group) => group.group_id.trim() || '未命名分组')
  if (!configuredGroups.length) return ''
  const shown = configuredGroups.slice(0, 3).join('、')
  const extra = configuredGroups.length > 3 ? ` 等 ${configuredGroups.length} 个分组` : ''
  return `已在分组 ${shown}${extra} 中配置接入。`
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
    submission_session_enabled: true,
    submission_session_required: false,
    submission_session_merge_text_to_first_message: false,
    quick_replies: [],
    review_shortcuts: [],
    global_shortcuts: [],
    agent_commands: [],
    agent_command_admins: [],
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
