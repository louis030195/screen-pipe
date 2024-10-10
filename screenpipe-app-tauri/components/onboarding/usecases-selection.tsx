import React from "react";
import { DialogHeader, DialogTitle } from "@/components/ui/dialog";
import {
  UserRound,
  CircleCheck,
  BriefcaseBusiness,
  Wrench,
  SlidersHorizontal,
} from "lucide-react";
import OnboardingNavigation from "@/components/onboarding/navigation";

interface OnboardingSelectionProps {
  className?: string;
  selectedOptions: string[] | null;
  handleOptionClick: (option: string) => void;
  handleNextSlide: () => void;
  handlePrevSlide: () => void;
}

const OPTIONS = [
  {
    key: "personalUse",
    icon: UserRound,
    label: "personal use",
    description: "daily summary & educational material organization",
  },
  {
    key: "professionalUse",
    icon: BriefcaseBusiness,
    label: "professional use",
    description: "productivity tracking & meeting summaries",
  },
  {
    key: "developmentlUse",
    icon: Wrench,
    label: "development purpose",
    description: "automate data capture & create ai-powered workflows",
  },
  {
    key: "otherUse",
    icon: SlidersHorizontal,
    label: "other",
    description: "",
  },
];

const SelectionItem: React.FC<{
  option: typeof OPTIONS[number];
  isSelected: boolean | undefined;
  onClick: () => void;
}> = ({ option, isSelected, onClick }) => {
  const { icon: Icon, label, description } = option;
  return (
    <div
      className={`w-[90%] flex items-center border prose prose-sm rounded-lg m-[10px] px-4 py-[10px] hover:bg-accent cursor-pointer
        ${isSelected ? "bg-primary text-primary-foreground transition duration-300 hover:bg-primary/90" : ""}`}
      onClick={onClick}
    >
      <span className="float-left">
        <Icon className="inline h-4 w-4 mr-2" />
        {label} {description && <span className="text-[12px]">({description})</span>}
      </span>
      {isSelected && <CircleCheck className="inline h-4 w-4 ml-auto" />}
    </div>
  );
};

const OnboardingSelection: React.FC<OnboardingSelectionProps> = ({
  className,
  selectedOptions,
  handleOptionClick,
  handleNextSlide,
  handlePrevSlide,
}) => {
  return (
    <div className={className}>
      <DialogHeader className="flex justify-center items-center">
        <div className="w-full !mt-[-10px] inline-flex justify-center">
          <img src="/128x128.png" alt="screenpipe-logo" width="72" height="72" />
        </div>
        <DialogTitle className="font-bold text-[30px] text-balance">
          what are you planning to use the screenpipe for?
        </DialogTitle>
      </DialogHeader>
      <div className="flex relative mt-8 justify-center items-center flex-col">
        <span className="text-[15px] w-full ml-24 text-left text-muted-foreground">
          you can select multiple options:
        </span>
        {OPTIONS.map((option) => (
          <SelectionItem
            key={option.key}
            option={option}
            isSelected={selectedOptions?.includes(option.key)}
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

export default OnboardingSelection;

