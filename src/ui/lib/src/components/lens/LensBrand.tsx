import Image from "next/image";
import lensSymbol from "../../assets/lens-symbol.svg";
import { cn } from "../../lib/cva.config";

export function LensBrand({ className }: { className?: string }) {
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-2", className)}>
      <Image src={lensSymbol} alt="" width={30} height={30} className="size-[30px]" loading="eager" unoptimized />
      <span className="flex items-baseline gap-1.5">
        <span className="text-xl leading-none font-semibold tracking-tight text-sidebar-foreground">Lens</span>
        <span className="text-[10px] font-medium text-muted-foreground">by LiteLLM</span>
      </span>
    </span>
  );
}
