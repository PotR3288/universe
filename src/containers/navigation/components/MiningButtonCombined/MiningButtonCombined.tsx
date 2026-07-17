import { Activity, useCallback } from 'react';
import { ButtonWrapper } from './styles.ts';
import { AnimatePresence } from 'motion/react';
import LoadingButton from './LoadingButton/LoadingButton.tsx';
import { useMiningStore } from '@app/store/useMiningStore.ts';
import { useMiningMetricsStore } from '@app/store/useMiningMetricsStore.ts';
import { useSetupStore } from '@app/store/useSetupStore.ts';
import { useConfigMiningStore } from '@app/store/useAppConfigStore.ts';
import { startMining } from '@app/store/actions/miningStoreActions.ts';
import MiningButton from './MiningButton/MiningButton.tsx';
import PlayIcon from './icons/PlayIcon.tsx';
import { setupStoreSelectors } from '@app/store/selectors/setupStoreSelectors.ts';

import MiningButtonPause from './MiningButton/components/pause/MiningButtonPause.tsx';
import useResumeCountdown from '@app/containers/navigation/components/MiningButtonCombined/useResumeCountdown.ts';
import ModeController from '@app/containers/navigation/components/MiningButtonCombined/MiningButton/components/ModeController.tsx';

export default function MiningButtonCombined() {
    const gpuMiningModuleInitialized = useSetupStore(setupStoreSelectors.isGpuMiningModuleInitialized);
    const cpuMiningModuleInitialized = useSetupStore(setupStoreSelectors.isCpuMiningModuleInitialized);
    const isMiningControlsEnabled = useMiningStore((s) => s.miningControlsEnabled);
    const changingModes = useMiningStore((s) => s.isChangingMode);
    const isMiningInitiated = useMiningStore((s) => s.isCpuMiningInitiated || s.isGpuMiningInitiated);
    const isCPUMining = useMiningMetricsStore((s) => s.cpu_mining_status.is_mining);
    const isGPUMining = useMiningMetricsStore((s) => s.gpu_mining_status.is_mining);
    const isMining = isCPUMining || isGPUMining;
    const cpuHashRate = useMiningMetricsStore((s) => s.cpu_mining_status.hash_rate);
    const gpuHashRate = useMiningMetricsStore((s) => s.gpu_mining_status.hash_rate);
    const combinedHashRate = cpuHashRate + gpuHashRate;
    const isCpuMiningEnabled = useConfigMiningStore((s) => s.cpu_mining_enabled);
    const isGpuMiningEnabled = useConfigMiningStore((s) => s.gpu_mining_enabled);
    const isMiningEnabled = isCpuMiningEnabled || isGpuMiningEnabled;
    // Show loading only when both flags indicate mining started but metrics
    // haven't arrived yet.  When the flags are out of sync in either direction,
    // treat it as a stable state — not a transient loading phase.  On multi-
    // instance systems (>64 threads) the backend's status aggregation loop may
    // stop before emitting a final `is_mining: false` update, leaving stale
    // metrics that would keep a spinner spinning forever.
    const isMiningLoading = isMiningInitiated && isMining && combinedHashRate === 0;
    const isMiningUnlocked = gpuMiningModuleInitialized || cpuMiningModuleInitialized;
    const isMiningButtonDisabled =
        isMiningLoading || !isMiningControlsEnabled || !isMiningEnabled || !isMiningUnlocked || changingModes;

    const resumeTime = useResumeCountdown();

    const handleStartMining = useCallback(async () => {
        await startMining();
    }, []);

    const controller = <ModeController />;

    const pauseButton = (
        <Activity mode={isMining ? 'visible' : 'hidden'} key="pause">
            <MiningButtonPause isMining={isMining} isMiningButtonDisabled={isMiningButtonDisabled}>
                {controller}
            </MiningButtonPause>
        </Activity>
    );
    const startButton = (
        <Activity mode={!isMining ? 'visible' : 'hidden'} key="start">
            <MiningButton
                buttonText={resumeTime.displayString ? 'resume' : 'start'}
                onClick={handleStartMining}
                disabled={isMiningButtonDisabled || isMiningLoading}
                icon={<PlayIcon />}
                isMining={isMining}
                resumeTime={resumeTime}
            >
                {controller}
            </MiningButton>
        </Activity>
    );

    const loader = (
        <Activity mode={isMiningLoading ? 'visible' : 'hidden'} key="loader">
            <LoadingButton />
        </Activity>
    );
    return (
        <ButtonWrapper>
            <AnimatePresence mode="popLayout">
                {loader}
                {startButton}
                {pauseButton}
            </AnimatePresence>
        </ButtonWrapper>
    );
}
