// Browser-only fixture; never an entry point of the desktop build.
import React from 'react';
import { createRoot } from 'react-dom/client';
import { Settings } from 'lucide-react';
import { api } from '@/api';
import { BackupSettingsForm } from '@/components/backup-settings';
import { Button } from '@/components/ui/button';
import { Toaster } from '@/components/ui/sonner';
import { Dialog, DialogTrigger, DialogContent, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { AlertDialog, AlertDialogTrigger, AlertDialogContent, AlertDialogTitle, AlertDialogDescription, AlertDialogCancel } from '@/components/ui/alert-dialog';
import { Sheet, SheetTrigger, SheetContent, SheetTitle, SheetDescription } from '@/components/ui/sheet';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from '@/components/ui/dropdown-menu';
import { ContextMenu, ContextMenuTrigger, ContextMenuContent, ContextMenuItem } from '@/components/ui/context-menu';
import { TooltipProvider, Tooltip, TooltipTrigger, TooltipContent } from '@/components/ui/tooltip';
import { Accordion, AccordionItem, AccordionTrigger, AccordionContent } from '@/components/ui/accordion';
import { Checkbox } from '@/components/ui/checkbox';
import { Progress } from '@/components/ui/progress';
import '@/style.css';

const config = { provider_type: 's3' as const, profile: 's3', region: 'us-east-1', path_style: true, endpoint: 'https://s3.example.com', prefix: 'fixture/', enabled: true, credentials_configured: true, protocol: 2, last_error: null };
const spaces = [{ ...config, space_id: 'one', library_id: 'library-one', bucket: 'space-one' }, { ...config, space_id: 'two', library_id: 'library-two', bucket: 'space-two' }];
const calls: string[] = [];
Object.assign(window, { fixtureCalls: calls });
Object.assign(api, {
  backupSettings: async () => spaces[0], backupSpaces: async () => spaces,
  backupSyncEnabled: async () => true,
  backupCredentials: async () => {
    if (location.search.includes('credential-error')) throw 'CREDENTIAL_ERROR';
    return { access_key_id: 'fixture-key', secret_access_key: 'fixture-secret' };
  },
  selectBackupSpace: async (id: string) => { calls.push(id); return spaces.find(s => s.space_id === id); },
});

function Fixture() {
  const [mounted, setMounted] = React.useState(true);
  return <TooltipProvider><Toaster position="top-center" /><div className="flex h-dvh flex-col gap-4 overflow-y-auto p-4">
    <div className="flex flex-wrap gap-4">
      <Button onClick={() => setMounted(v => !v)}>切换备份表单</Button>
      <Tooltip><TooltipTrigger asChild><Button aria-label="SVG 设置图标"><Settings className="size-4" /></Button></TooltipTrigger><TooltipContent>图标说明</TooltipContent></Tooltip>
      <Dialog><DialogTrigger asChild><Button>打开 Dialog</Button></DialogTrigger><DialogContent><DialogTitle>Dialog 测试</DialogTitle><DialogDescription>验证滚动锁与定位</DialogDescription></DialogContent></Dialog>
      <AlertDialog><AlertDialogTrigger asChild><Button>打开 AlertDialog</Button></AlertDialogTrigger><AlertDialogContent><AlertDialogTitle>AlertDialog 测试</AlertDialogTitle><AlertDialogDescription>验证取消和焦点</AlertDialogDescription><AlertDialogCancel>取消</AlertDialogCancel></AlertDialogContent></AlertDialog>
      <Sheet><SheetTrigger asChild><Button>打开 Sheet</Button></SheetTrigger><SheetContent><SheetTitle>Sheet 测试</SheetTitle><SheetDescription>验证侧边浮层</SheetDescription></SheetContent></Sheet>
      <DropdownMenu><DropdownMenuTrigger asChild><Button>打开 DropdownMenu</Button></DropdownMenuTrigger><DropdownMenuContent><DropdownMenuItem>菜单项目</DropdownMenuItem></DropdownMenuContent></DropdownMenu>
      <ContextMenu><ContextMenuTrigger asChild><Button>打开 ContextMenu</Button></ContextMenuTrigger><ContextMenuContent><ContextMenuItem>右键菜单项目</ContextMenuItem></ContextMenuContent></ContextMenu>
    </div>
    <Accordion type="single" collapsible><AccordionItem value="one"><AccordionTrigger>展开内容</AccordionTrigger><AccordionContent>手风琴内容</AccordionContent></AccordionItem></Accordion>
    <Checkbox aria-label="测试复选框" /><Progress variant="circular" value={42} aria-label="圆形进度" />
    {mounted && <BackupSettingsForm saved={() => {}} />}
  </div></TooltipProvider>;
}
createRoot(document.getElementById('root')!).render(<React.StrictMode><Fixture /></React.StrictMode>);
