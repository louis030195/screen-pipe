{/*TODO: setup screenpipe status  <10-10-24, @tribhuwan-kumar>*/}

import React from "react";
import { ArrowUpRight } from "lucide-react";
import { open } from "@tauri-apps/plugin-shell";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import OnboardingNavigation from "@/components/onboarding/navigation";
import { spinner } from "@/components/spinner";

interface OnboardingStatusProps {
  className?: string;
  handlePrevSlide: () => void;
  handleNextSlide: () => void;
}

const OnboardingStatus: React.FC<OnboardingStatusProps> = ({
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
          wait we are checking the status of screenpipe
        </DialogTitle>
        <h1 className="font-medium text-center !mt-[-1px] text-md prose">
          hold on
        </h1>
      </DialogHeader>
      <svg
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        viewBox="0 0 24 24"
        strokeLinecap="round"
        strokeLinejoin="round"
        xmlns="http://www.w3.org/2000/svg"
        className="size-5 animate-spin stroke-zinc-400"
      >
        <path d="M12 3v3m6.366-.366-2.12 2.12M21 12h-3m.366 6.366-2.12-2.12M12 21v-3m-6.366.366 2.12-2.12M3 12h3m-.366-6.366 2.12 2.12"></path>
      </svg>
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

export default OnboardingStatus;

