import React from "react";
import { HelpCircle, Info } from "lucide-react";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import OnboardingNavigation from "@/components/onboarding/navigation";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

interface OnboardingInstructionsProps {
  className?: string;
  handlePrevSlide: () => void;
  handleNextSlide: () => void;
}

const OnboardingInstructions: React.FC<OnboardingInstructionsProps> = ({
  className = "",
  handlePrevSlide,
  handleNextSlide,
}) => {

  return (
    <div className={`${className} w-full flex justify-center flex-col`}>
      <DialogHeader className="px-2">
        <div className="w-full inline-flex !mt-[-10px] justify-center">
          <img
            src="/128x128.png"
            alt="screenpipe-logo"
            width="72"
            height="72"
          />
        </div>
        <DialogTitle className="text-center !mt-[-3px] font-bold text-[30px] text-balance flex justify-center">
          screenpipe usage instructions
        </DialogTitle>
        <h1 className="text-center !mt-[-1px] text-lg">
          before we begin, learn how to use screenpipe effectively!
        </h1>
      </DialogHeader>
      <div className="flex justify-center">
        <div className="h-[1px] w-40 rounded-full bg-gradient-to-l from-slate-500/50 to-transparent"></div>
        <div className="h-[1px] w-40 rounded-full bg-gradient-to-r from-slate-500/50 to-transparent"></div>
      </div>
      <div className="mt-2 w-full flex justify-around flex-col">
        <div className="mx-4 mb-2">
          <h2 className="font-semibold text-md">
            search functionality:
          </h2>
          <p className="text-muted-foreground text-[14px]">
            <span className="font-medium text-nowrap text-[14px] prose mr-1">
              on screenpipe&apos;s main menu,
            </span> 
            you&apos;ll find a search bar to query the 24/7 captured data. 
            it filters out pii and lets you select specific results for 
            ai summarization and processing, providing valuable insights.
          </p>
        </div>
				<div className="flex justify-center">
					<div className="h-[1px] w-[80%] rounded-full bg-gradient-to-l from-slate-500/50 to-transparent"></div>
					<div className="h-[1px] w-[80%] rounded-full bg-gradient-to-r from-slate-500/50 to-transparent"></div>
				</div>
        <div className="mx-4 mb-2">
          <h2 className="font-medium text-md">
            status menu:
          </h2>
          <p className="text-muted-foreground text-[14px]">
            <span className="font-medium text-nowrap text-[14px] prose mr-1">
             screenpipe status menu serves  
            </span> 
              as a health and process monitor and allows you to 
              adjust advanced preferences for backend processes
          </p>
        </div>
				<div className="flex justify-center">
					<div className="h-[1px] w-[80%] rounded-full bg-gradient-to-l from-slate-500/50 to-transparent"></div>
					<div className="h-[1px] w-[80%] rounded-full bg-gradient-to-r from-slate-500/50 to-transparent"></div>
				</div>
        <div className="mx-4 mb-2">
          <h2 className="font-medium text-md">
            settings menu:
          </h2>
          <p className="text-muted-foreground text-[14px]">
            <span className="font-medium text-nowrap text-[14px] prose mr-1">
              the settings menu,
            </span> 
            is where you can configure various options like monitor
            recorder settings and ai preferences. this section lets you customize how
            screenpipe interacts with llms
          </p>
        </div>
				<div className="flex justify-center">
					<div className="h-[1px] w-40 rounded-full bg-gradient-to-l from-slate-500/50 to-transparent"></div>
					<div className="h-[1px] w-40 rounded-full bg-gradient-to-r from-slate-500/50 to-transparent"></div>
				</div>
        <div className="mx-4">
          <h2 className="font-medium text-md">
            screenpipe pipestore:
          </h2>
          <p className="text-muted-foreground text-[14px]">
            <span className="font-medium text-nowrap text-[14px] prose mr-1">
              the pipestore in screenpipe
            </span> 
              allows users to browse and download various functionality-enhancing pipes.
              explore different plugins to customize workflows, and extend screenpipe’s capabilities. 
          </p>
        </div>
      </div>
      <span className="absolute bottom-12 text-muted-foreground prose-sm text-center block w-full ">
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger asChild>
              <span>
                <Info className="inline w-4 h-4 mb-[1.5px] mr-1" />hover on these 
                &apos;<HelpCircle className="mb-1 inline h-4 w-4 cursor-default" />&apos;
                for more info
              </span>
            </TooltipTrigger>
            <TooltipContent side="right">
              <a
              onClick={() =>
                open(
                  "https://docs.screenpi.pe/",
                )
              }
              href="#"
              className="mt-4 text-muted-foreground text-sm mr-auto ml-auto !text-center hover:underline"
            >
              checkout our docs to learn more
            </a>
            </TooltipContent>
          </Tooltip>
          </TooltipProvider>
      </span>
      <OnboardingNavigation
        className="mt-8"
        handlePrevSlide={handlePrevSlide}
        handleNextSlide={handleNextSlide}
        prevBtnText="previous"
        nextBtnText="next"
      />
    </div>
  );
};

export default OnboardingInstructions;

