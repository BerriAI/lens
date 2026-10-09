import Image from "next/image";
import lensLogoDark from "../../assets/lens-logo-dark.png";
import lensLogoLight from "../../assets/lens-logo-light.png";
import { cn } from "../../lib/cva.config";

export function LensBrand({ className }: { className?: string }) {
  return (
    <span className={cn("block w-[132px] shrink-0", className)}>
      <Image
        src={lensLogoLight}
        alt="LiteLLM Lens"
        width={1290}
        height={360}
        className="block h-auto w-full dark:hidden"
        loading="eager"
        unoptimized
      />
      <Image
        src={lensLogoDark}
        alt="LiteLLM Lens"
        width={1290}
        height={360}
        className="hidden h-auto w-full dark:block"
        loading="eager"
        unoptimized
      />
    </span>
  );
}
