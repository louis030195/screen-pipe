import React from 'react';
import { Card, CardContent } from "@/components/ui/card";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import OnboardingNavigation from '@/components/onboarding/navigation';
import { Info } from "lucide-react";

interface OnboardingExperimentalFeaturesProps {
  className?: string,
  handlePrevSlide: () => void;
  handleNextSlide: () => void;
}

const OnboardingExperimentalFeatures: React.FC<OnboardingExperimentalFeaturesProps> = ({ 
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
        <DialogTitle className="text-center !mt-[-2px] font-bold text-[32px] text-balance flex justify-center">
          features of screenpipe
        </DialogTitle>
        <h1 className="font-medium text-center !mt-[-1px] text-md prose">
          screenpipe hasn&apos;t been extensively tested. we&apos;d love your feedback!
        </h1>
      </DialogHeader>
      <div className="mt-3 w-full flex justify-around flex-row">
        <Card className="w-[45%]">
          <CardContent className="mt-4">
            <h1 className="font-semibold text-lg">essential features:</h1>
              <ul className="list-disc ml-5">
                <li className="text-muted-foreground text-sm">
                  <span className="font-medium text-nowrap text-[14px] prose mr-1">
                      continuous media capture: 
                  </span>
                  seamlessly records screen and audio data 24/7, storing everything locally
                </li>
                <li className="text-muted-foreground text-sm">
                  <span className="font-medium text-nowrap text-[14px] prose mr-1">
                    personalized ai integration:
                  </span>
                    empowers ai models with insights derived from your captured data.
                </li>
                <li className="text-muted-foreground text-sm">
                  <span className="font-medium text-[14px] prose mr-1">
                    open source &amp; secure:
                  </span>
                    ensures your data remains private, giving you full control over storage and processing.
                </li>
              </ul>
          </CardContent>
        </Card>
        <Card className="w-[45%]">
          <CardContent className="mt-4">
            <h1 className="font-semibold text-lg">experimental features:</h1>
              <ul className="list-disc ml-5">
                <li className="text-muted-foreground text-sm">
                  <span className="font-medium text-nowrap text-[14px] prose mr-1">
                    remove personal information (pii):
                  </span>
                    for automatically detecting and removing your personally identifiable information (pii) from captured data to ensure your privacy  with data
                    protection.
                </li>
                <li className="text-muted-foreground text-sm mt-2">
                  <span className="font-medium text-nowrap text-[14px] prose mr-1">
                    restart interval:
                  </span>
                    it&apos;s an experimental setting that lets you set an automatic restart interval to refresh processes or clear cached data, ensuring optimized performance.
                </li>
              </ul>
          </CardContent>
        </Card>
      </div>
      <span className="absolute bottom-12 text-muted-foreground prose-sm text-center block w-full z-[-1]">
       <Info className="inline w-4 h-4 mb-[1px]" /> be careful with experimental features!
      </span>
      <OnboardingNavigation 
        className="mt-8"
        handlePrevSlide={handlePrevSlide}
        handleNextSlide={handleNextSlide}
        prevBtnText="previous"
        nextBtnText="let&apos;s get started"
      />
    </div>
  );
} 

export default OnboardingExperimentalFeatures;

