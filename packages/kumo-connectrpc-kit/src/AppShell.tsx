import { Button, Sidebar, useSidebar } from "@cloudflare/kumo";
import { ListIcon, SignOutIcon } from "@phosphor-icons/react";
import type { Icon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { useAuth } from "./auth.js";

/** One sidebar nav entry. The consumer owns routing — `onClick` navigates. */
export interface AppShellNavItem {
  label: string;
  icon: Icon;
  active?: boolean;
  onClick: () => void;
}

export interface AppShellProps {
  /** Brand element shown at the top of the sidebar (logo + name). */
  brand: ReactNode;
  /** Sidebar nav entries — routing is app-specific; visibility is the app's
   *  call (filter by the Cedar perms on `useAuth().state.whoami` before passing). */
  nav: AppShellNavItem[];
  /** Label above the nav group. */
  navGroupLabel?: string;
  /** Called after the shared logout runs (e.g. redirect to /login). Optional. */
  onSignedOut?: () => void;
  signOutLabel?: string;
  children: ReactNode;
}

function MenuButton(): ReactNode {
  const { setOpenMobile, isMobile } = useSidebar();
  if (!isMobile) return null;
  return (
    <Button
      variant="secondary"
      size="sm"
      onClick={() => setOpenMobile(true)}
      aria-label="Open menu"
      className="md:hidden mb-4 shrink-0"
    >
      <ListIcon size={16} />
      Menu
    </Button>
  );
}

/**
 * Presentational app shell: Kumo sidebar (brand + nav + sign-out) wrapping the
 * page. Fully decoupled — nav, brand and sign-out come in as props, so any
 * ConnectRPC+Kumo app reuses it with its own routing and auth.
 */
export function AppShell({
  brand,
  nav,
  navGroupLabel = "Workspace",
  onSignedOut,
  signOutLabel = "Sign out",
  children,
}: AppShellProps): ReactNode {
  // The ONE shared logout — couples the shell to the kit's auth, by design.
  const { logout } = useAuth();
  const handleSignOut = (): void => {
    logout();
    onSignedOut?.();
  };
  return (
    <Sidebar.Provider defaultOpen>
      <div className="flex h-full w-full bg-kumo-base">
        <Sidebar>
          <Sidebar.Content>
            <Sidebar.Group>
              <Sidebar.GroupLabel>{brand}</Sidebar.GroupLabel>
            </Sidebar.Group>
            <Sidebar.Group>
              <Sidebar.GroupLabel>{navGroupLabel}</Sidebar.GroupLabel>
              <Sidebar.Menu>
                {nav.map((item) => (
                  <Sidebar.MenuButton
                    key={item.label}
                    icon={item.icon}
                    active={item.active}
                    onClick={item.onClick}
                  >
                    {item.label}
                  </Sidebar.MenuButton>
                ))}
              </Sidebar.Menu>
            </Sidebar.Group>
          </Sidebar.Content>
          <Sidebar.Footer>
            <Sidebar.Menu>
              <Sidebar.MenuButton icon={SignOutIcon} onClick={handleSignOut}>
                {signOutLabel}
              </Sidebar.MenuButton>
            </Sidebar.Menu>
          </Sidebar.Footer>
        </Sidebar>
        <main className="flex-1 overflow-y-auto p-6">
          <MenuButton />
          {children}
        </main>
      </div>
    </Sidebar.Provider>
  );
}
