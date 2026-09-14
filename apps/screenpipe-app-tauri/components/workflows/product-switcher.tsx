// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { Check, ChevronDown, ScanLine, Workflow } from "lucide-react";
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

export type ProductMode = "screenpipe" | "workflows";
export function ProductSwitcher({ mode, onChange, compact = false }: {
  mode: ProductMode;
  onChange: (mode: ProductMode) => void;
  compact?: boolean;
}) {
  return <DropdownMenu>
    <DropdownMenuTrigger asChild>
      <button type="button" aria-label="Switch workspace" className={cn("flex items-center gap-2 rounded-md px-3 py-2 text-inherit hover:bg-foreground/5 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring", compact ? "text-sm" : "w-full text-lg font-semibold")}>
        {mode === "workflows" ? <Workflow size={18} /> : <ScanLine size={18} />}
        <span>{mode === "workflows" ? "Workflows" : "Screenpipe"}</span><ChevronDown size={14} className="ml-auto text-muted-foreground" />
      </button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="start" sideOffset={6} className="w-72 p-1.5" aria-label="Workspaces">
      {([
        ["screenpipe", ScanLine, "Screenpipe", "Find anything from your day"],
        ["workflows", Workflow, "Screenpipe Workflows", "Understand how your work gets done"],
      ] as const).map(([value, Icon, title, subtitle]) => <DropdownMenuItem key={value} onSelect={() => onChange(value)} className="gap-3 px-3 py-3 cursor-pointer" role="menuitemradio" aria-checked={mode === value}>
        <Icon size={18} className="shrink-0" /><span className="flex-1"><span className="block text-sm font-medium">{title}</span><span className="block mt-1 text-xs text-muted-foreground">{subtitle}</span></span>{mode === value && <Check size={15} />}
      </DropdownMenuItem>)}
    </DropdownMenuContent>
  </DropdownMenu>;
}
