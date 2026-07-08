import { type FormEvent, type ReactNode, useEffect, useState } from 'react'
import { Button, Card, Input, Spinner, Toast } from '@heroui/react'
import {
  BarChart3,
  Eye,
  FileText,
  LayoutGrid,
  LogOut,
  PanelLeftClose,
  PanelLeftOpen,
  Send,
  Settings2,
  ShieldCheck,
  type LucideIcon,
} from 'lucide-react'
import { api } from './api/client'
import { MeResponse } from './api/types'
import { REVIEW_STAGE_OPTIONS, type ToastKind, type ViewKey, showToast } from './shared'
import { PostsView } from './views/PostsView'
import { AgentView, SettingsView } from './views/SettingsView'
import { TagMappingView } from './views/TagMappingView'
import { StatsView } from './views/StatsView'

type NavItem = {
  key: ViewKey
  label: string
  Icon: LucideIcon
  globalOnly?: boolean
}

const NAV_ITEMS: NavItem[] = [
  { key: 'review', label: '审核', Icon: Eye },
  { key: 'sent', label: '已发送', Icon: Send },
  { key: 'stats', label: '统计', Icon: BarChart3 },
  { key: 'agent', label: 'Agent', Icon: FileText, globalOnly: true },
  { key: 'settings', label: '设置', Icon: Settings2 },
  { key: 'tag-mapping', label: '标签映射', Icon: LayoutGrid },
]

function App() {
  const [me, setMe] = useState<MeResponse | null>(null)
  const [authChecked, setAuthChecked] = useState(false)
  const [view, setView] = useState<ViewKey>('review')
  const [mobileNavOpen, setMobileNavOpen] = useState(false)
  const navItems = NAV_ITEMS.filter((item) => !item.globalOnly || me?.role === 'global_admin')
  const activeNavItem = navItems.find((item) => item.key === view) ?? navItems[0]
  const navClass = (key: ViewKey) => `nav-button ${view === key ? 'is-active' : ''}`

  useEffect(() => {
    api<MeResponse>('/auth/me')
      .then(setMe)
      .catch(() => setMe(null))
      .finally(() => setAuthChecked(true))
  }, [])

  useEffect(() => {
    if (me?.role !== 'global_admin' && view === 'agent') {
      setView('review')
    }
  }, [me?.role, view])

  useEffect(() => {
    const mediaQuery = window.matchMedia('(max-width: 980px)')
    const closeOnDesktop = () => {
      if (!mediaQuery.matches) {
        setMobileNavOpen(false)
      }
    }

    closeOnDesktop()
    mediaQuery.addEventListener('change', closeOnDesktop)
    return () => mediaQuery.removeEventListener('change', closeOnDesktop)
  }, [])

  useEffect(() => {
    if (!mobileNavOpen) {
      return
    }

    const previousOverflow = document.body.style.overflow
    document.body.style.overflow = 'hidden'

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setMobileNavOpen(false)
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => {
      document.body.style.overflow = previousOverflow
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [mobileNavOpen])

  async function logout() {
    await api('/auth/logout', { method: 'POST' }).catch(() => undefined)
    setMe(null)
    setMobileNavOpen(false)
  }

  function selectView(nextView: ViewKey) {
    setView(nextView)
    setMobileNavOpen(false)
    window.requestAnimationFrame(() => {
      window.scrollTo({ top: 0, left: 0, behavior: 'auto' })
    })
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
      <div className={`app-shell ${mobileNavOpen ? 'is-mobile-nav-open' : ''}`}>
        <div className="mobile-nav-rail" aria-label="主导航快捷入口">
          <button
            className="mobile-nav-rail-action"
            type="button"
            aria-label="展开导航"
            aria-controls="app-sidebar"
            aria-expanded={mobileNavOpen}
            onClick={() => setMobileNavOpen(true)}
          >
            <PanelLeftOpen size={20} />
          </button>
          <div className="mobile-nav-rail-tabs" role="navigation" aria-label="主导航快捷入口">
            {navItems.map(({ key, label, Icon }) => (
              <button
                key={key}
                className={`mobile-nav-rail-tab ${view === key ? 'is-active' : ''}`}
                type="button"
                aria-label={label}
                aria-current={view === key ? 'page' : undefined}
                title={label}
                onClick={() => selectView(key)}
              >
                <Icon size={20} />
              </button>
            ))}
          </div>
          <span className="mobile-nav-rail-label">{activeNavItem?.label}</span>
        </div>

        <button
          className="mobile-nav-backdrop"
          type="button"
          aria-label="收起导航"
          aria-hidden={!mobileNavOpen}
          tabIndex={mobileNavOpen ? 0 : -1}
          onClick={() => setMobileNavOpen(false)}
        />

        <aside id="app-sidebar" className="sidebar" aria-label="主导航">
          <div className="sidebar-head">
            <Brand />
            <Button
              className="sidebar-close-button"
              variant="tertiary"
              isIconOnly
              aria-label="收起导航"
              onClick={() => setMobileNavOpen(false)}
            >
              <PanelLeftClose size={18} />
            </Button>
          </div>
          <nav className="nav" aria-label="主导航">
            {navItems.map(({ key, label, Icon }) => (
              <Button
                key={key}
                className={navClass(key)}
                variant={view === key ? 'primary' : 'tertiary'}
                fullWidth
                aria-current={view === key ? 'page' : undefined}
                onClick={() => selectView(key)}
              >
                <Icon size={18} />
                {label}
              </Button>
            ))}
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

function HeroShell({ children }: { children: ReactNode }) {
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
      <span className="brand-mark" aria-hidden="true">O</span>
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
            <label className="field-stack">
              <span className="field-label">用户名</span>
              <Input
                placeholder="请输入用户名"
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                autoComplete="username"
              />
            </label>
            <label className="field-stack">
              <span className="field-label">密码</span>
              <Input
                placeholder="请输入密码"
                type="password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                autoComplete="current-password"
              />
            </label>
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

export default App
