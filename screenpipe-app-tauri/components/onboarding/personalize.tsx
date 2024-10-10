import React from "react";
import { Card, CardContent } from "@/components/ui/card";
import { TextSearch, BotMessageSquare } from "lucide-react";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import OnboardingNavigation from "@/components/onboarding/navigation";

interface OnboardingPersonalizeProps {
  handleOptionClick: (option: string) => void;
  handleNextSlide: () => void;
  handlePrevSlide: () => void;
  selectedPersonalization?: string | null;
  className?: string;
}

const PERSONALIZATION_OPTIONS = [
  {
    key: "withoutAI",
    icon: TextSearch,
    title: "conventional search",
    description: "seamless functionality, easily monitor screen and audio with built-in ocr for precise scanning.",
    note: "no api key needed.",
  },
  {
    key: "withAI",
    icon: BotMessageSquare,
    title: "ai-enhanced Search",
    description: "leverage ai capabilities and seamlessly monitor your screen, using advanced AI to summarize collected data.",
    note: "api key required.",
  },
];

const CardItem: React.FC<{
  option: typeof PERSONALIZATION_OPTIONS[number];
  isSelected: boolean;
  onClick: () => void;
}> = ({ option, isSelected, onClick }) => {
  const { icon: Icon, title, description, note } = option;

  return (
    <div className="relative group h-[270px]">
      <div className={`absolute h-full !mt-[-5px] inset-0 rounded-lg transition-all duration-300 ease-out group-hover:before:opacity-100 group-hover:before:scale-100 
        before:absolute before:inset-0 before:rounded-lg before:border-2 before:border-black dark:before:border-white before:opacity-0 before:scale-95 before:transition-all 
        before:duration-300 before:ease-out ${isSelected ? "before:!border-none" : "" }`}
      />
      <Card
        className={`p-4 h-full !mt-[-5px] cursor-pointer bg-white dark:bg-gray-800 hover:bg-accent transition-all relative z-[1] duration-300 ease-out group-hover:scale-[0.98]
        ${isSelected ? "bg-accent transition-transform relative border-2 border-black dark:border-white" : "" }`}
        onClick={onClick}
      >
        <CardContent className="flex flex-col w-[250px] justify-center">
          <Icon className="w-16 h-16 mx-auto" />
          <h2 className="font-semibold text-xl text-center mt-1">{title}</h2>
          <span className="prose prose-sm mt-1">{description}</span>
          <span className="text-muted-foreground text-center prose-sm mt-4">{note}</span>
        </CardContent>
      </Card>
    </div>
  );
};

const OnboardingPersonalize: React.FC<OnboardingPersonalizeProps> = ({
  className = "",
  selectedPersonalization = "",
  handleOptionClick,
  handleNextSlide,
  handlePrevSlide,
}) => {
  return (
    <div className={`${className} w-full flex justify-center flex-col relative`}>
      <DialogHeader className="mt-1 px-2">
        <div className="w-full !mt-[-10px] inline-flex justify-center">
          <img src="/128x128.png" alt="screenpipe-logo" width="72" height="72" />
        </div>
        <DialogTitle className="text-center !mt-[-2px] font-bold text-[32px] text-balance flex justify-center">
          personalize your screenpipe
        </DialogTitle>
        <p className="text-center text-lg">
          how would you like to use screenpipe?
        </p>
      </DialogHeader>
      <div className="flex w-full justify-around mt-6">
        {PERSONALIZATION_OPTIONS.map((option) => (
          <CardItem
            key={option.key}
            option={option}
            isSelected={selectedPersonalization === option.key}
            onClick={() => handleOptionClick(option.key)}
          />
        ))}
      </div>
      <OnboardingNavigation
        className="mt-9"
        handlePrevSlide={handlePrevSlide}
        handleNextSlide={handleNextSlide}
        prevBtnText="previous"
        nextBtnText="next"
      />
    </div>
  );
};

export default OnboardingPersonalize;

